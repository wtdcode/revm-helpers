use alloy::{
    primitives::{Address, U256},
    sol_types::SolCall,
};
use revm::{
    Inspector,
    context::ContextTr,
    database::CacheDB,
    interpreter::{CallInputs, CallOutcome, CallScheme, Gas, InstructionResult, InterpreterResult},
    primitives::{B256, keccak256},
    state::{AccountInfo, Bytecode},
};

use crate::{call::EVMTestingTxBuilder, rand_db::RandDB};

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
    B256::from_slice(&random_u256(rand).as_le_slice())
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
    use revm::primitives::{B256, keccak256};

    use crate::proxy::{detect_balance_of_slot, detect_proxy_slot};

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
