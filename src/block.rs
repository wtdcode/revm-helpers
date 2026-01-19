use alloy::{consensus::transaction::Recovered, network::primitives::HeaderResponse};
use alloy_chains::Chain;
use revm::{
    context::{BlockEnv, CfgEnv, TxEnv},
    primitives::{TxKind, U256, hardfork::SpecId},
};

pub fn empty_blockenv() -> BlockEnv {
    BlockEnv::default()
}

pub fn testing_env(chain: Chain) -> CfgEnv {
    let mut cfg_env = CfgEnv::default();
    cfg_env.chain_id = chain.into();
    cfg_env.disable_base_fee = true;
    cfg_env.disable_block_gas_limit = true;
    cfg_env.spec = SpecId::default();

    cfg_env
}

pub fn block_env_from_rpc<H>(block_header: &H) -> BlockEnv
where
    H: HeaderResponse,
{
    let mut env = empty_blockenv();
    env.beneficiary = block_header.beneficiary();
    env.difficulty = block_header.difficulty();
    env.gas_limit = block_header.gas_limit();
    env.number = U256::from(block_header.number());
    env.prevrandao = block_header.mix_hash();
    env.timestamp = U256::from(block_header.timestamp());
    env.basefee = block_header.base_fee_per_gas().unwrap_or_default();

    env
}

pub fn tx_env_from_rpc<T>(tx: &Recovered<T>) -> TxEnv
where
    T: alloy::consensus::Transaction,
{
    let mut env = TxEnv::default();
    env.caller = tx.signer();
    env.nonce = tx.nonce();
    env.gas_limit = tx.gas_limit();
    env.gas_price = if tx.is_legacy() {
        tx.gas_price().unwrap_or_default()
    } else {
        tx.max_fee_per_gas()
    };
    env.gas_priority_fee = tx.max_priority_fee_per_gas();
    env.max_fee_per_blob_gas = tx.max_fee_per_blob_gas().unwrap_or_default();
    env.data = tx.input().clone();
    env.kind = if let Some(target) = tx.to() {
        TxKind::Call(target)
    } else {
        TxKind::Create
    };
    env.chain_id = tx.chain_id();
    env.tx_type = tx.ty();
    env.value = tx.value();
    env.access_list = tx.access_list().map(|v| v.clone()).unwrap_or_default();
    env.authorization_list = tx
        .authorization_list()
        .map(|v| v.iter().map(|v| Err(v.clone()).into()).collect())
        .unwrap_or_default();
    env.blob_hashes = tx
        .blob_versioned_hashes()
        .map(|v| v.to_vec())
        .unwrap_or_default();
    env
}
