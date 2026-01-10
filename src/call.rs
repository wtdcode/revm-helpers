use alloy::{
    primitives::{Address, U256},
    sol_types::SolCall,
};
use alloy_chains::Chain;
use color_eyre::eyre::eyre;
use revm::{
    Context, Database, DatabaseCommit, DatabaseRef, ExecuteEvm, Inspector, MainBuilder,
    MainContext,
    context::{
        BlockEnv, CfgEnv, TransactionType, TxEnv,
        result::{EVMError, ExecResultAndState, ExecutionResult, Output},
        tx::TxEnvBuilder,
    },
    interpreter::interpreter::EthInterpreter,
};
use revm::{inspector::InspectEvm, primitives::TxKind};
use revm_inspectors::tracing::{CallTraceArena, TracingInspector, TracingInspectorConfig};

#[derive(Debug, Default, Clone)]
pub struct EVMCall {
    pub tx: TxEnv,
    pub block: BlockEnv,
    pub cfg: CfgEnv,
}

impl EVMCall {
    pub fn tx(mut self, tx: TxEnv) -> Self {
        self.tx = tx;
        self
    }

    pub fn block(mut self, block: BlockEnv) -> Self {
        self.block = block;
        self
    }

    pub fn cfg(mut self, cfg: CfgEnv) -> Self {
        self.cfg = cfg;
        self
    }

    pub fn call<T>(
        self,
        db: T,
    ) -> Result<ExecResultAndState<ExecutionResult>, EVMError<<T as Database>::Error>>
    where
        T: DatabaseCommit + Database + DatabaseRef,
    {
        let context = Context::mainnet()
            .with_db(db)
            .with_cfg(self.cfg.clone())
            .with_block(self.block);

        let mut evm = context.build_mainnet();

        evm.transact(self.tx)
    }

    pub fn inspect<T, INSP>(
        self,
        db: T,
        insp: INSP,
    ) -> Result<ExecResultAndState<ExecutionResult>, EVMError<<T as Database>::Error>>
    where
        T: DatabaseCommit + Database + DatabaseRef,
        INSP: Inspector<Context<BlockEnv, TxEnv, CfgEnv, T>, EthInterpreter>,
    {
        let context = Context::mainnet()
            .with_db(db)
            .with_cfg(self.cfg.clone())
            .with_block(self.block);

        let mut evm = context.build_mainnet_with_inspector(insp);

        evm.inspect_tx(self.tx)
    }

    pub fn trace_inspect<T, INSP>(
        self,
        db: T,
        insp: INSP,
    ) -> Result<
        (ExecResultAndState<ExecutionResult>, CallTraceArena),
        EVMError<<T as Database>::Error>,
    >
    where
        T: DatabaseCommit + Database + DatabaseRef,
        INSP: Inspector<Context<BlockEnv, TxEnv, CfgEnv, T>, EthInterpreter>,
    {
        let trace = TracingInspector::new(TracingInspectorConfig::all());
        let mut insp = (insp, trace);
        let result = self.inspect(db, &mut insp)?;
        let traces = insp.1.into_traces();
        Ok((result, traces))
    }

    pub fn trace<T>(
        self,
        db: T,
    ) -> Result<
        (ExecResultAndState<ExecutionResult>, CallTraceArena),
        EVMError<<T as Database>::Error>,
    >
    where
        T: DatabaseCommit + Database + DatabaseRef,
    {
        let mut insp = TracingInspector::new(TracingInspectorConfig::all());
        let result = self.inspect(db, &mut insp)?;
        let traces = insp.into_traces();
        Ok((result, traces))
    }

    pub fn deploy<T>(
        self,
        mut db: T,
        target: Option<Address>,
    ) -> Result<Address, color_eyre::Report>
    where
        T: DatabaseCommit + Database + DatabaseRef,
        color_eyre::Report: From<<T as Database>::Error>,
    {
        let mut r = self.call(&mut db).map_err(|e| eyre!("deploy err {}", e))?;
        match r.result {
            ExecutionResult::Success {
                reason: _,
                gas_used: _,
                gas_refunded: _,
                logs: _,
                output,
            } => match output {
                Output::Create(_v, addr) => {
                    let result = if let Some(target) = target {
                        let original = addr.expect("No address returned??");
                        let account = r.state.remove(&original).expect("No deployment??");
                        r.state.insert(target, account);
                        target
                    } else {
                        addr.expect("No address returned??")
                    };
                    db.commit(r.state);
                    Ok(result)
                }
                _ => {
                    tracing::error!(output = ?output, "failed to deploy");
                    Err(eyre!("fail to deploy though tx succeeeds"))
                }
            },
            _ => Err(eyre!("Fail to deploy the initial bot contract due to: {:?}", &r)),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct EVMTestingTxBuilder {
    pub caller: Address,
    pub nonce: u64,
    pub chain: Chain,
}

impl EVMTestingTxBuilder {
    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }
    pub fn nonce(mut self, nonce: u64) -> Self {
        self.nonce = nonce;
        self
    }
    pub fn chain(mut self, chain: Chain) -> Self {
        self.chain = chain;
        self
    }
    pub fn mainnet(self) -> Self {
        self.chain(Chain::mainnet())
    }
    pub fn build_deploy(&self, code_and_args: Vec<u8>) -> EVMCall {
        let tx = TxEnvBuilder::new()
            .tx_type(Some(TransactionType::Eip1559.into()))
            .caller(self.caller)
            .kind(TxKind::Create)
            .gas_limit(u64::MAX)
            .gas_price(0)
            .gas_priority_fee(Some(0))
            .value(U256::ZERO)
            .data(code_and_args.into())
            .nonce(self.nonce)
            .chain_id(Some(self.chain.into()))
            .build()
            .expect("deploy tx");
        EVMCall::default().tx(tx)
    }

    pub fn build_low_level_call(&self, target: Address, data: Vec<u8>, value: U256) -> EVMCall {
        let tx = TxEnvBuilder::new()
            .tx_type(Some(TransactionType::Eip1559.into()))
            .caller(self.caller)
            .kind(TxKind::Call(target))
            .gas_limit(u64::MAX)
            .gas_price(0)
            .gas_priority_fee(Some(0))
            .value(value)
            .data(data.into())
            .nonce(self.nonce)
            .chain_id(Some(self.chain.into()))
            .build()
            .expect("call tx");
        EVMCall::default().tx(tx)
    }

    pub fn build_sol_call<C>(&self, target: Address, call: C, value: U256) -> EVMCall
    where
        C: SolCall,
    {
        let data = call.abi_encode();
        self.build_low_level_call(target, data, value)
    }
}
