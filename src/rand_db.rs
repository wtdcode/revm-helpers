use std::{cell::RefCell, collections::BTreeMap, convert::Infallible, ops::DerefMut};

use fast_rands::Rand;
use revm::{
    Database, DatabaseRef,
    primitives::{Address, B256, StorageKey, StorageValue, U256, keccak256},
    state::{AccountInfo, Bytecode},
};

// Keep high bits clear to fit into most cases
pub fn random_low_storage<R: Rand>(rng: &mut R) -> U256 {
    let val = rng.next();
    U256::from_be_bytes(
        Address::from_word(keccak256(val.to_be_bytes()))
            .into_word()
            .0,
    )
}

pub struct RandDBInner {
    pub rand: fast_rands::RomuDuoJrRand,
    pub storages: BTreeMap<Address, BTreeMap<U256, U256>>,
    pub storages_reverse_mapping: BTreeMap<U256, (Address, U256)>,
}

pub struct RandDB {
    inner: RefCell<RandDBInner>,
}

impl RandDB {
    pub fn new(rand: fast_rands::RomuDuoJrRand) -> Self {
        Self {
            inner: RefCell::new(RandDBInner {
                rand,
                storages: BTreeMap::new(),
                storages_reverse_mapping: BTreeMap::new(),
            }),
        }
    }

    pub fn into_inner(self) -> RandDBInner {
        self.inner.into_inner()
    }
}

impl DatabaseRef for RandDB {
    type Error = Infallible;
    fn basic_ref(&self, _address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(Some(AccountInfo::default()))
    }

    fn storage_ref(
        &self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        let mut inner = self.inner.borrow_mut();
        let inner_mut = inner.deref_mut();

        let val = *inner_mut
            .storages
            .entry(address)
            .or_default()
            .entry(index)
            .or_insert_with(|| {
                let next_value = random_low_storage(&mut inner_mut.rand);
                inner_mut
                    .storages_reverse_mapping
                    .insert(next_value, (address, index));
                next_value
            });
        Ok(val)
    }
    fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
        Ok(B256::ZERO)
    }
    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(Bytecode::default())
    }
}

impl Database for RandDB {
    type Error = Infallible;
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.basic_ref(address)
    }
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.block_hash_ref(number)
    }
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.code_by_hash_ref(code_hash)
    }
    fn storage(
        &mut self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        self.storage_ref(address, index)
    }
}
