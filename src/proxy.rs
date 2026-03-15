use alloy::{
    primitives::{Address, U256},
    sol_types::SolCall,
};
use revm::{
    Database, DatabaseRef, Inspector,
    context::{BlockEnv, ContextTr},
    database::CacheDB,
    interpreter::{CallInputs, CallOutcome, CallScheme, Gas, InstructionResult, InterpreterResult},
    primitives::{B256, keccak256},
    state::{AccountInfo, Bytecode},
};
use tracing::instrument;

use crate::{
    call::EVMTestingTxBuilder,
    rand_db::{LogDB, RandDB, StorageAccess, random_low_storage},
};

pub struct ProxyInspector {
    storage_address: Address,
    initial_input: Vec<u8>,
    pub delegate: Option<CallInputs>,
}

impl ProxyInspector {
    pub fn new(storage_address: Address, initial_input: Vec<u8>) -> Self {
        Self {
            storage_address,
            initial_input,
            delegate: None,
        }
    }
}

impl<CTX: ContextTr> Inspector<CTX> for ProxyInspector {
    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.input.bytes(ctx) == self.initial_input
            && inputs.target_address == self.storage_address
            && matches!(inputs.scheme, CallScheme::DelegateCall)
        {
            self.delegate = Some(inputs.clone());
            Some(CallOutcome::new(
                InterpreterResult::new(InstructionResult::Revert, [].into(), Gas::new(0)),
                0..0,
            ))
        } else {
            None
        }
    }
}

const MAGIC_INPUT: &[u8] = &alloy::hex!("0102030405060708090a0b0c0d0e0f");

pub(crate) fn random_u256<R: fast_rands::Rand>(rand: &mut R) -> U256 {
    let v0 = rand.next();
    let v1 = rand.next();
    let v2 = rand.next();
    let v3 = rand.next();
    U256::from_limbs([v0, v1, v2, v3])
}

pub(crate) fn random_b256<R: fast_rands::Rand>(rand: &mut R) -> B256 {
    B256::from_slice(random_u256(rand).as_le_slice())
}

fn prepare_env(code: &[u8]) -> Option<(Address, Address, CacheDB<RandDB>)> {
    let mut rand = fast_rands::RomuDuoJrRand::new();
    let random_source_address = Address::from_word(random_b256(&mut rand));
    let random_target_address = Address::from_word(random_b256(&mut rand));
    let mut db = CacheDB::new(RandDB::new(rand));
    let bytecode = Bytecode::new_raw_checked(code.iter().copied().collect()).ok()?;
    let hash = bytecode.hash_slow();
    db.insert_account_info(
        random_target_address,
        AccountInfo::new(U256::ZERO, 0, hash, bytecode),
    );
    Some((random_source_address, random_target_address, db))
}

pub fn detect_proxy_slot(code: &[u8]) -> Option<U256> {
    let (random_source_address, random_target_address, mut db) = prepare_env(code)?;
    let mut insp = ProxyInspector::new(random_target_address, MAGIC_INPUT.to_vec());
    let call = EVMTestingTxBuilder::default()
        .caller(random_source_address)
        .mainnet()
        .nonce(0)
        .build_low_level_call(random_target_address, MAGIC_INPUT.to_vec(), U256::ZERO)
        .gas_limit(524280)
        .inspect(&mut db, &mut insp);
    match call {
        Ok(_result) => {
            if let Some(delegate) = insp.delegate {
                let target = delegate.bytecode_address;
                let records = db.db.into_inner();
                let storage_value = U256::from_be_bytes(target.into_word().0);
                // TODO: Maybe iterating all storages?
                records
                    .storages_reverse_mapping
                    .get(&storage_value)
                    .map(|v| v.1)
            } else {
                None
            }
        }
        Err(e) => {
            tracing::warn!("detect_proxy_slot error with {}", e);
            None
        }
    }
}

alloy::sol!(
    contract ERC20 {
        function balanceOf(address owner) public returns (uint);
    }
);

fn call_balance_of_db<DB: Database>(
    db: &mut DB,
    caller: Address,
    account: Address,
    token: Address,
    block: BlockEnv,
) -> Option<U256> {
    let call = EVMTestingTxBuilder::default()
        .caller(caller)
        .build_sol_call(token, ERC20::balanceOfCall { owner: account }, U256::ZERO)
        .block(block)
        .testing_main()
        .gas_limit(524280)
        .call(db);
    let result = match call {
        Ok(result) => result,
        Err(e) => {
            tracing::debug!("failed to call due to {}", e);
            return None;
        }
    };

    tracing::debug!(
        "Result is {:?}, length is {:?}",
        &result.result,
        result.result.output().map(|v| v.len())
    );
    ERC20::balanceOfCall::abi_decode_returns(result.result.output()?).ok()
}

fn choose_candidate_value<R: fast_rands::Rand>(rand: &mut R, access: StorageAccess) -> U256 {
    loop {
        let value = random_low_storage(rand);
        if value != access.value {
            return value;
        }
    }
}

#[instrument(name = "detect_slot", skip(db, block))]
pub fn detect_account_balance_of_slot_db<DB: Database + DatabaseRef>(
    account: Address,
    token: Address,
    block: BlockEnv,
    db: DB,
) -> Option<U256> {
    let mut rand = fast_rands::RomuDuoJrRand::new();
    let random_source_address = Address::from_word(random_b256(&mut rand));
    let mut db = LogDB::new(db);
    let _initial_value = call_balance_of_db(
        &mut db,
        random_source_address,
        account,
        token,
        block.clone(),
    )?;
    let (db, storage_accesses) = db.into_parts();

    for access in storage_accesses.into_iter().rev() {
        let value_set = choose_candidate_value(&mut rand, access);
        let mut replay_db = CacheDB::new(&db);
        if let Err(e) = replay_db.insert_account_storage(access.address, access.index, value_set) {
            tracing::debug!("fail to insert db due to {}", e);
            continue;
        }

        let value = call_balance_of_db(
            &mut replay_db,
            random_source_address,
            account,
            token,
            block.clone(),
        );

        if value == Some(value_set) {
            return Some(access.index);
        }
    }

    None
}

pub fn detect_account_balance_of_slot(account: Address, code: &[u8]) -> Option<U256> {
    let (random_source_address, random_target_address, mut db) = prepare_env(code)?;
    let call = EVMTestingTxBuilder::default()
        .caller(random_source_address)
        .mainnet()
        .nonce(0)
        .build_sol_call(
            random_target_address,
            ERC20::balanceOfCall { owner: account },
            U256::ZERO,
        )
        .gas_limit(524280)
        .call(&mut db);
    let Ok(result) = call else {
        return None;
    };

    let value = ERC20::balanceOfCall::abi_decode_returns(result.result.output()?).ok()?;
    let mut rand_db = db.db.into_inner();
    let balance_slot = rand_db.storages_reverse_mapping.get(&value).map(|v| v.1)?;

    // try set and check again
    let (random_source_address, random_target_address, mut db) = prepare_env(code)?;
    let value_set = random_low_storage(&mut rand_db.rand);
    db.insert_account_storage(random_target_address, balance_slot, value_set)
        .ok()?;

    let call = EVMTestingTxBuilder::default()
        .caller(random_source_address)
        .mainnet()
        .nonce(0)
        .build_sol_call(
            random_target_address,
            ERC20::balanceOfCall { owner: account },
            U256::ZERO,
        )
        .gas_limit(524280)
        .call(&mut db);
    let Ok(result) = call else {
        return None;
    };
    let value = ERC20::balanceOfCall::abi_decode_returns(result.result.output()?).ok()?;
    if value_set == value {
        Some(balance_slot)
    } else {
        None
    }
}

pub fn detect_balance_of_slot(code: &[u8]) -> Option<U256> {
    let (random_source_address, random_target_address, mut db) = prepare_env(code)?;
    let call = EVMTestingTxBuilder::default()
        .caller(random_source_address)
        .mainnet()
        .nonce(0)
        .build_sol_call(
            random_target_address,
            ERC20::balanceOfCall {
                owner: random_source_address,
            },
            U256::ZERO,
        )
        .gas_limit(524280)
        .call(&mut db);
    match call {
        Ok(result) => {
            let value = ERC20::balanceOfCall::abi_decode_returns(result.result.output()?).ok()?;
            let records = db.db.into_inner();
            let balance_slot = records.storages_reverse_mapping.get(&value).map(|v| v.1)?;
            for mapping_slot in 0..0xffu64 {
                let bs = random_source_address
                    .into_word()
                    .into_iter()
                    .chain(U256::from(mapping_slot).to_be_bytes_vec().into_iter())
                    .collect::<Vec<_>>();
                let address_balance_slot = keccak256(&bs);
                if U256::from_be_bytes(address_balance_slot.0) == balance_slot {
                    return Some(U256::from(mapping_slot));
                }
            }
            None
        }
        Err(e) => {
            tracing::warn!("detect_proxy_slot error with {}", e);
            None
        }
    }
}

#[cfg(test)]
mod test {
    macro_rules! test_token_slot {
        ($test_name:ident, $address:literal, $expected_slot:literal) => {
            #[test]
            fn $test_name() {
                // Load and decode the bytecode relative to the current file
                let bytecode = alloy::hex::decode(include_str!(concat!("codes/", $address)))
                    .expect("Failed to decode hex string from file");

                // Assert that no proxy is detected
                assert_eq!(
                    detect_proxy_slot(&bytecode),
                    None,
                    "Expected detect_proxy_slot to return None"
                );

                // Detect balance slot and assert expected value
                let balance_slot =
                    detect_balance_of_slot(&bytecode).expect("Failed to detect balance_of slot");

                assert_eq!(
                    balance_slot,
                    alloy::primitives::uint!($expected_slot),
                    "Balance slot did not match expected value"
                );
            }
        };
    }
    use alloy::{
        network::BlockResponse,
        providers::{Provider, ProviderBuilder},
        uint,
    };
    use revm::{
        database::{AlloyDB, CacheDB, WrapDatabaseAsync},
        primitives::{B256, address, b256, keccak256},
    };

    use crate::{
        block::block_env_from_rpc,
        proxy::{
            detect_account_balance_of_slot, detect_account_balance_of_slot_db,
            detect_balance_of_slot, detect_proxy_slot,
        },
    };

    #[test]
    fn test_usdc_account_balance_of() {
        let usdc_proxy = alloy::hex::decode(include_str!(
            "codes/0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
        ))
        .unwrap();
        let usdc_impl = alloy::hex::decode(include_str!(
            "codes/0x43506849d7c04f9138d1a2050bbf3a0c054402dd"
        ))
        .unwrap();
        let proxy_slot = detect_proxy_slot(&usdc_proxy).unwrap();
        assert_eq!(
            B256::from_slice(&proxy_slot.to_be_bytes_vec()),
            keccak256(b"org.zeppelinos.proxy.implementation")
        );
        let balanceof_slot = detect_account_balance_of_slot(
            address!("0xeE21222E88a745f1FF89D4AC69a77CEcb7974eBb"),
            &usdc_impl,
        );
        assert_eq!(
            balanceof_slot,
            Some(uint!(
                26844865903416581458858788339750883127623690146227240090174371479474754231843_U256
            ))
        );
    }

    #[test]
    fn test_usdc() {
        let usdc_proxy = alloy::hex::decode(include_str!(
            "codes/0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
        ))
        .unwrap();
        let usdc_impl = alloy::hex::decode(include_str!(
            "codes/0x43506849d7c04f9138d1a2050bbf3a0c054402dd"
        ))
        .unwrap();
        let proxy_slot = detect_proxy_slot(&usdc_proxy).unwrap();
        assert_eq!(
            B256::from_slice(&proxy_slot.to_be_bytes_vec()),
            keccak256(b"org.zeppelinos.proxy.implementation")
        );
        let balanceof_slot = detect_balance_of_slot(&usdc_impl).unwrap();
        assert_eq!(balanceof_slot, alloy::primitives::uint!(0x9_U256));
    }

    #[test]
    fn test_paxg() {
        let paxg_proxy = alloy::hex::decode(include_str!(
            "codes/0x45804880De22913dAFE09f4980848ECE6EcbAf78"
        ))
        .unwrap();
        let paxg_impl = alloy::hex::decode(include_str!(
            "codes/0x74271f2282ed7ee35c166122a60c9830354be42a"
        ))
        .unwrap();
        let proxy_slot = detect_proxy_slot(&paxg_proxy).unwrap();
        assert_eq!(
            B256::from_slice(&proxy_slot.to_be_bytes_vec()),
            b256!("0x7050c9e0f4ca769c69bd3a8ef740bc37934f8e2c036e5a723fd8ee048ed3f8c3")
        );
        let balanceof_slot = detect_balance_of_slot(&paxg_impl).unwrap();
        assert_eq!(balanceof_slot, alloy::primitives::uint!(0x1_U256));
    }

    #[test]
    fn test_vyper() {
        let code = alloy::hex::decode(include_str!(
            "codes/0xf939e0a03fb07f59a73314e73794be0e57ac1b4e"
        ))
        .unwrap();
        let balance_slot = detect_account_balance_of_slot(
            address!("0xf6f1fE2C6FDC68EC3731Eb4A90fCb91D987486E9"),
            &code,
        )
        .unwrap();
        let expected =
            uint!(0x26f45eb385cd8316267595472e71ba7da7e8bafe0511964afa8457d6f8b20e2f_U256);
        assert_eq!(balance_slot, expected);
    }

    #[ignore]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_wlfi() {
        let sub = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::EnvFilter::builder()
                    .with_default_directive(tracing::Level::DEBUG.into())
                    .from_env()
                    .expect("env contains non-utf8"),
            )
            .finish();
        tracing::subscriber::set_global_default(sub).unwrap();
        let rpc =
            std::env::var("ETH_RPC_URL").unwrap_or_else(|_| "http://10.96.179.48:8565".to_string());
        let rpc = ProviderBuilder::new().connect(&rpc).await.unwrap();
        let number = rpc.get_block_number().await.unwrap();
        let block = block_env_from_rpc(
            rpc.get_block_by_number(number.into())
                .await
                .unwrap()
                .unwrap()
                .header(),
        );
        let db = CacheDB::new(WrapDatabaseAsync::new(AlloyDB::new(rpc, number.into())).unwrap());
        let wlfi_proxy = alloy::hex::decode(include_str!(
            "codes/0xdA5e1988097297dCdc1f90D4dFE7909e847CBeF6"
        ))
        .unwrap();
        let proxy_slot = detect_proxy_slot(&wlfi_proxy).unwrap();
        assert_eq!(
            B256::from_slice(&proxy_slot.to_be_bytes_vec()),
            b256!("0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc")
        );
        let balanceof_slot = detect_account_balance_of_slot_db(
            address!("0xeE21222E88a745f1FF89D4AC69a77CEcb7974eBb"),
            address!("0xdA5e1988097297dCdc1f90D4dFE7909e847CBeF6"),
            block,
            db,
        )
        .unwrap();
        assert_eq!(
            balanceof_slot,
            alloy::primitives::uint!(
                26383648194240856301554830195144840311273963909714824038045424366773278612897_U256
            )
        );
    }

    #[test]
    fn test_uni() {
        let code = alloy::hex::decode(include_str!(
            "codes/0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984"
        ))
        .unwrap();
        let balance_slot = detect_account_balance_of_slot(
            address!("0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984"),
            &code,
        )
        .unwrap();
        let expected = uint!(
            55561402736569575101087086774556051094108371327836379602443489089635694777977_U256
        );
        assert_eq!(balance_slot, expected);
    }

    test_token_slot!(
        test_usdt,
        "0xdac17f958d2ee523a2206206994597c13d831ec7",
        0x2_U256
    );

    test_token_slot!(
        test_weth,
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
        0x3_U256
    );
}
