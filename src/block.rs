use alloy::network::{TransactionResponse, primitives::HeaderResponse};
use alloy_chains::Chain;
use revm::{
    context::{BlockEnv, CfgEnv},
    primitives::{U256, hardfork::SpecId},
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

pub fn block_env_from_rpc<T, H>(block: alloy::rpc::types::Block<T, H>) -> BlockEnv
where
    H: HeaderResponse,
    T: TransactionResponse,
{
    let mut env = empty_blockenv();
    env.beneficiary = block.header.beneficiary();
    env.difficulty = block.header.difficulty();
    env.gas_limit = block.header.gas_limit();
    env.number = U256::from(block.header.number());
    env.prevrandao = block.header.mix_hash();
    env.timestamp = U256::from(block.header.timestamp());
    env.basefee = block.header.base_fee_per_gas().unwrap_or_default();

    env
}
