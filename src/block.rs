use alloy_chains::Chain;
use revm::{
    context::{BlockEnv, CfgEnv},
    primitives::hardfork::SpecId,
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
