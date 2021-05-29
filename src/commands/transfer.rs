use crate::commands::{request_status_sign, sign};
use crate::lib::{
    environment::Environment,
    get_idl_string,
    nns_types::account_identifier::AccountIdentifier,
    nns_types::icpts::{ICPTs, TRANSACTION_FEE},
    nns_types::{Memo, SendArgs},
    DfxResult, LEDGER_CANISTER_ID,
};
use anyhow::anyhow;
use candid::Encode;
use clap::Clap;
use ic_types::principal::Principal;
use std::str::FromStr;

const SEND_METHOD: &str = "send_dfx";

/// Transfer ICP from the user to the destination AccountIdentifier
#[derive(Default, Clap)]
pub struct TransferOpts {
    /// AccountIdentifier of transfer destination.
    pub to: String,

    /// ICPs to transfer to the destination AccountIdentifier
    /// Can be specified as a Decimal with the fractional portion up to 8 decimal places
    /// i.e. 100.012
    #[clap(long, validator(icpts_amount_validator))]
    pub amount: Option<String>,

    /// Specify ICP as a whole number, helpful for use in conjunction with `--e8s`
    #[clap(long, validator(e8s_validator), conflicts_with("amount"))]
    pub icp: Option<String>,

    /// Specify e8s as a whole number, helpful for use in conjunction with `--icp`
    #[clap(long, validator(e8s_validator), conflicts_with("amount"))]
    pub e8s: Option<String>,

    /// Specify a numeric memo for this transaction.
    #[clap(long, validator(memo_validator))]
    pub memo: Option<String>,

    /// Transaction fee, default is 10000 e8s.
    #[clap(long, validator(icpts_amount_validator))]
    pub fee: Option<String>,
}

pub async fn exec(env: &dyn Environment, opts: TransferOpts) -> DfxResult<String> {
    let amount = get_icpts_from_args(opts.amount, opts.icp, opts.e8s)?;
    let fee = opts.fee.map_or(Ok(TRANSACTION_FEE), |v| {
        ICPTs::from_str(&v).map_err(|err| anyhow!(err))
    })?;
    let memo = Memo(opts.memo.unwrap_or("0".to_string()).parse::<u64>().unwrap());
    let to = AccountIdentifier::from_str(&opts.to).map_err(|err| anyhow!(err))?;
    let canister_id = Principal::from_text(LEDGER_CANISTER_ID)?;

    let args = Encode!(&SendArgs {
        memo,
        amount,
        fee,
        from_subaccount: None,
        to,
        created_at_time: None,
    })?;

    let argument = Some(get_idl_string(
        &args,
        &canister_id.clone().to_string(),
        SEND_METHOD,
        "args",
        "raw",
    )?);
    let opts = sign::SignOpts {
        canister_id: canister_id.clone().to_string(),
        method_name: SEND_METHOD.to_string(),
        query: false,
        update: true,
        argument,
        r#type: Some("raw".to_string()),
    };
    let msg_with_req_id = sign::exec(env, opts).await?;
    let request_id: String = msg_with_req_id
        .request_id
        .expect("No request id for transfer call found")
        .into();
    let req_status_signed_msg = request_status_sign::exec(
        env,
        request_status_sign::RequestStatusSignOpts {
            request_id: format!("0x{}", request_id),
            canister_id: canister_id.to_string(),
        },
    )
    .await?;

    let mut out = String::new();
    out.push_str("{ \"ingress\": ");
    out.push_str(&msg_with_req_id.buffer);
    out.push_str(", \"request_status\": ");
    out.push_str(&req_status_signed_msg);
    out.push_str("}");

    Ok(out)
}

fn get_icpts_from_args(
    amount: Option<String>,
    icp: Option<String>,
    e8s: Option<String>,
) -> DfxResult<ICPTs> {
    if amount.is_none() {
        let icp = match icp {
            Some(s) => {
                // validated by e8s_validator
                let icps = s.parse::<u64>().unwrap();
                ICPTs::from_icpts(icps).map_err(|err| anyhow!(err))?
            }
            None => ICPTs::from_e8s(0),
        };
        let icp_from_e8s = match e8s {
            Some(s) => {
                // validated by e8s_validator
                let e8s = s.parse::<u64>().unwrap();
                ICPTs::from_e8s(e8s)
            }
            None => ICPTs::from_e8s(0),
        };
        let amount = icp + icp_from_e8s;
        Ok(amount.map_err(|err| anyhow!(err))?)
    } else {
        Ok(ICPTs::from_str(&amount.unwrap())
            .map_err(|err| anyhow!("Could not add ICPs and e8s: {}", err))?)
    }
}

fn e8s_validator(e8s: &str) -> Result<(), String> {
    if e8s.parse::<u64>().is_ok() {
        return Ok(());
    }
    Err("Must specify a non negative whole number.".to_string())
}

fn icpts_amount_validator(icpts: &str) -> Result<(), String> {
    ICPTs::from_str(icpts).map(|_| ())
}

fn memo_validator(memo: &str) -> Result<(), String> {
    if memo.parse::<u64>().is_ok() {
        return Ok(());
    }
    Err("Must specify a non negative whole number.".to_string())
}
