use crate::lib::environment::Environment;
use crate::lib::error::DfxResult;
use crate::lib::sign::sign_transport::SignReplicaV2Transport;
use crate::lib::sign::signed_message::SignedMessageV1;
use crate::util::get_local_candid_path;
use crate::util::{blob_from_arguments, get_candid_type};
use anyhow::{anyhow, bail};
use candid::{CandidType, Decode, Deserialize};
use chrono::Utc;
use clap::Clap;
use humanize_rs::duration;
use ic_agent::AgentError;
use ic_types::principal::Principal as CanisterId;
use ic_types::principal::Principal;
use ic_utils::interfaces::management_canister::builders::{CanisterInstall, CanisterSettings};
use ic_utils::interfaces::management_canister::MgmtMethod;
use std::option::Option;
use std::str::FromStr;
use std::time::SystemTime;

/// Sign a canister call and generate message file in json
#[derive(Clap)]
pub struct SignOpts {
    /// Specifies the name of the canister to call.
    pub canister_name: String,

    /// Specifies the method name to call on the canister.
    pub method_name: String,

    /// Sends a query request to a canister.
    #[clap(long)]
    pub query: bool,

    /// Sends an update request to a canister. This is the default if the method is not a query method.
    #[clap(long, conflicts_with("query"))]
    pub update: bool,

    /// Specifies the argument to pass to the method.
    pub argument: Option<String>,

    /// Specifies the config for generating random argument.
    #[clap(long, conflicts_with("argument"))]
    pub random: Option<String>,

    /// Specifies the data type for the argument when making the call using an argument.
    #[clap(long, requires("argument"), possible_values(&["idl", "raw"]))]
    pub r#type: Option<String>,

    /// Specifies how long will the message be valid in seconds, default to be 300s (5 minutes)
    #[clap(long, default_value("5m"))]
    pub expire_after: String,

    /// Specifies the output file name.
    #[clap(long, default_value("message.json"))]
    pub file: String,
}

pub async fn exec(env: &dyn Environment, opts: SignOpts) -> DfxResult {
    let callee_canister = opts.canister_name.as_str();
    let method_name = opts.method_name.as_str();

    let canister_id =
        Principal::from_text(callee_canister).expect("Coouldn't convert canister id to principal");
    let candid_path = get_local_candid_path(canister_id.clone());

    let method_type = candid_path.and_then(|path| get_candid_type(&path, method_name));
    let is_query_method = match &method_type {
        Some((_, f)) => Some(f.is_query()),
        None => None,
    };

    let is_query = match is_query_method {
        Some(true) => !opts.update,
        Some(false) => {
            if opts.query {
                bail!(
                    "Invalid method call: {} is not a query method.",
                    method_name
                );
            } else {
                false
            }
        }
        None => opts.query,
    };

    // Get the argument, get the type, convert the argument to the type and return
    // an error if any of it doesn't work.
    let arg_value = {
        let arguments = opts.argument.as_deref();
        let arg_type = opts.r#type.as_deref();
        blob_from_arguments(arguments, opts.random.as_deref(), arg_type, &method_type)?
    };
    let agent = env
        .get_agent()
        .ok_or_else(|| anyhow!("Cannot get HTTP client from environment."))?;

    let network = env
        .get_network_descriptor()
        .expect("Cannot get network descriptor from environment.")
        .providers
        .first()
        .expect("Cannot get network provider (url).")
        .to_string();

    let sender = env
        .get_selected_identity_principal()
        .expect("Selected identity not instantiated.");

    let timeout = duration::parse(&opts.expire_after)
        .map_err(|_| anyhow!("Cannot parse expire_after as a duration (e.g. `1h`, `1h 30m`)"))?;
    //let timeout = Duration::from_secs(opts.expire_after);
    let expiration_system_time = SystemTime::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow!("Time wrapped around."))?;
    let chorono_timeout = chrono::Duration::seconds(timeout.as_secs() as i64);
    let creation = Utc::now();
    let expiration = creation
        .checked_add_signed(chorono_timeout)
        .ok_or_else(|| anyhow!("Expiration datetime overflow."))?;

    let message_template = SignedMessageV1::new(
        creation,
        expiration,
        network,
        sender,
        canister_id.clone(),
        method_name.to_string(),
        arg_value.clone(),
    );

    let file_name = opts.file;

    let mut sign_agent = agent.clone();
    sign_agent.set_transport(SignReplicaV2Transport::new(file_name, message_template));

    let is_management_canister = canister_id == Principal::management_canister();
    let effective_canister_id = get_effective_canister_id(
        is_management_canister,
        method_name,
        &arg_value,
        canister_id.clone(),
    )?;

    if is_query {
        let res = sign_agent
            .query(&canister_id, method_name)
            .with_effective_canister_id(effective_canister_id)
            .with_arg(&arg_value)
            .expire_at(expiration_system_time)
            .call()
            .await;
        match res {
            Err(AgentError::TransportError(b)) => {
                println!("{}", b);
                Ok(())
            }
            Err(e) => bail!(e),
            Ok(_) => unreachable!(),
        }
    } else {
        let res = sign_agent
            .update(&canister_id, method_name)
            .with_effective_canister_id(effective_canister_id)
            .with_arg(&arg_value)
            .expire_at(expiration_system_time)
            .call()
            .await;
        match res {
            Err(AgentError::TransportError(b)) => {
                println!("{}", b);
                Ok(())
            }
            Err(e) => bail!(e),
            Ok(_) => unreachable!(),
        }
    }
}

pub fn get_effective_canister_id(
    is_management_canister: bool,
    method_name: &str,
    arg_value: &[u8],
    canister_id: CanisterId,
) -> DfxResult<CanisterId> {
    if is_management_canister {
        let method_name = MgmtMethod::from_str(method_name).map_err(|_| {
            anyhow!(
                "Attempted to call an unsupported management canister method: {}",
                method_name
            )
        })?;
        match method_name {
            MgmtMethod::CreateCanister | MgmtMethod::RawRand => {
                bail!(format!("{} can only be called via an inter-canister call. Try calling this without `--no-wallet`.",
                    method_name.as_ref()))
            }
            MgmtMethod::InstallCode => {
                let install_args = candid::Decode!(arg_value, CanisterInstall)?;
                Ok(install_args.canister_id)
            }
            MgmtMethod::UpdateSettings => {
                #[derive(CandidType, Deserialize)]
                struct In {
                    canister_id: CanisterId,
                    settings: CanisterSettings,
                }
                let in_args = candid::Decode!(arg_value, In)?;
                Ok(in_args.canister_id)
            }
            MgmtMethod::StartCanister
            | MgmtMethod::StopCanister
            | MgmtMethod::CanisterStatus
            | MgmtMethod::DeleteCanister
            | MgmtMethod::DepositCycles
            | MgmtMethod::UninstallCode
            | MgmtMethod::ProvisionalTopUpCanister => {
                #[derive(CandidType, Deserialize)]
                struct In {
                    canister_id: CanisterId,
                }
                let in_args = candid::Decode!(arg_value, In)?;
                Ok(in_args.canister_id)
            }
            MgmtMethod::ProvisionalCreateCanisterWithCycles => {
                Ok(CanisterId::management_canister())
            }
        }
    } else {
        Ok(canister_id)
    }
}