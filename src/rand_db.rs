use std::{cell::RefCell, collections::BTreeMap, convert::Infallible, ops::DerefMut};

use fast_rands::Rand;
use revm::{
    Database, DatabaseRef,
    primitives::{Address, B256, StorageKey, StorageValue, U256},
    state::{AccountInfo, Bytecode},
};

// Keep high bits clear to fit into most cases
pub fn random_low_storage<R: Rand>(rng: &mut R) -> U256 {
    let val = rng.next();
    U256::from(val)
}

pub struct RandDBInner {
    pub rand: fast_rands::RomuDuoJrRand,
    pub storages: BTreeMap<Address, BTreeMap<U256, U256>>,
    pub storages_reverse_mapping: BTreeMap<U256, (Address, U256)>,
}

pub struct RandDB {
    inner: RefCell<RandDBInner>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageAccess {
    pub address: Address,
    pub index: U256,
    pub value: U256,
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
        tracing::debug!("Random db generates {} for {}::{}", val, address, index);
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

pub struct LogDB<DB> {
    db: DB,
    storage_accesses: RefCell<Vec<StorageAccess>>,
}

impl<DB> LogDB<DB> {
    pub fn new(db: DB) -> Self {
        Self {
            db,
            storage_accesses: RefCell::new(Vec::new()),
        }
    }

    pub fn into_parts(self) -> (DB, Vec<StorageAccess>) {
        (self.db, self.storage_accesses.into_inner())
    }

    fn record_storage_access(&self, address: Address, index: StorageKey, value: StorageValue) {
        self.storage_accesses.borrow_mut().push(StorageAccess {
            address,
            index,
            value,
        });
    }
}

impl<DB: DatabaseRef> DatabaseRef for LogDB<DB> {
    type Error = DB::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.basic_ref(address)
    }

    fn storage_ref(
        &self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        let value = self.db.storage_ref(address, index)?;
        self.record_storage_access(address, index, value);
        Ok(value)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.db.block_hash_ref(number)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.code_by_hash_ref(code_hash)
    }
}

impl<DB: Database> Database for LogDB<DB> {
    type Error = DB::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.basic(address)
    }

    fn storage(
        &mut self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        let value = self.db.storage(address, index)?;
        self.record_storage_access(address, index, value);
        Ok(value)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.db.block_hash(number)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.code_by_hash(code_hash)
    }
}

pub struct TargetedRandDB<DB> {
    pub target: Address,
    pub db: DB,
    pub rand_db: RandDB,
}

impl<DB> TargetedRandDB<DB> {
    pub fn new(target: Address, fallback: DB, rand_db: RandDB) -> Self {
        Self {
            target,
            db: fallback,
            rand_db,
        }
    }
}

impl<DB: DatabaseRef> DatabaseRef for TargetedRandDB<DB> {
    type Error = DB::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.basic_ref(address)
    }
    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.db.block_hash_ref(number)
    }
    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.code_by_hash_ref(code_hash)
    }
    fn storage_ref(
        &self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        if address == self.target {
            Ok(self.rand_db.storage_ref(address, index).unwrap())
        } else {
            self.db.storage_ref(address, index)
        }
    }
}

impl<DB: Database> Database for TargetedRandDB<DB> {
    type Error = DB::Error;
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.basic(address)
    }
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.db.block_hash(number)
    }
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.code_by_hash(code_hash)
    }
    fn storage(
        &mut self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        if address == self.target {
            Ok(self.rand_db.storage(address, index).unwrap())
        } else {
            self.db.storage(address, index)
        }
    }
}
