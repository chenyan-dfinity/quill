use crate::{
    commands::transfer,
    lib::{
        environment::Environment,
        nns_types::account_identifier::{AccountIdentifier, Subaccount},
        DfxResult,
    },
};
use clap::Clap;
use ic_types::Principal;
use serde::{Deserialize, Serialize};

/// Creates a neuron with the specified amount of ICPs
#[derive(Clap)]
pub struct TransferOpts {
    /// ICPs to be staked on the newly created neuron.
    #[clap(long)]
    amount: String,

    /// The name of the neuron (up to 8 ASCII characters).
    #[clap(long, validator(neuron_name_validator))]
    name: String,

    /// Transaction fee, default is 10000 e8s.
    #[clap(long)]
    fee: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CreateNeuronMessage {
    transfer: crate::lib::sign::signed_message::SignedMessageV1,
    claim_or_refersh: bool,
}

pub async fn exec(env: &dyn Environment, opts: TransferOpts) -> DfxResult {
    let controller = crate::commands::principal::get_principal(env)?;
    let nonce = convert_name_to_memo(&opts.name);
    let neuron_subaccount = get_neuron_subaccount(&controller, nonce);
    transfer::exec(
        env,
        transfer::TransferOpts {
            to: AccountIdentifier::new(controller, Some(neuron_subaccount)).to_hex(),
            amount: Some(opts.amount),
            fee: opts.fee,
            memo: Some(nonce.to_string()),
            ..Default::default()
        },
    )
    .await
}

fn get_neuron_subaccount(controller: &Principal, nonce: u64) -> Subaccount {
    use openssl::sha::Sha256;
    let mut data = Sha256::new();
    data.update(&[0x0c]);
    data.update(b"neuron-stake");
    data.update(&controller.as_slice());
    data.update(&nonce.to_be_bytes());
    Subaccount(data.finish())
}

fn convert_name_to_memo(name: &str) -> u64 {
    let mut bytes = std::collections::VecDeque::from(name.as_bytes().to_vec());
    while bytes.len() < 8 {
        bytes.push_front(0)
    }
    let mut arr: [u8; 8] = [0; 8];
    arr.copy_from_slice(&bytes.into_iter().collect::<Vec<_>>());
    u64::from_be_bytes(arr)
}

fn neuron_name_validator(name: &str) -> Result<(), String> {
    // Convert to bytes before checking the length to restrict it ot ASCII only
    if name.as_bytes().len() > 8 {
        return Err("The neuron name must be 8 character or less".to_string());
    }
    Ok(())
}