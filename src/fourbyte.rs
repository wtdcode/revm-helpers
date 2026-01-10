use alloy::{
    json_abi::{Event, Function},
    primitives::{B256, Selector},
};

#[auto_impl::auto_impl(&, Arc, Rc)]
pub trait FourBytesProvider: Send + Sync {
    fn function_signatures(
        &self,
        selector: Selector,
    ) -> impl Future<Output = Result<Vec<Function>, color_eyre::Report>>;
    fn event_signatures(
        &self,
        topic: B256,
    ) -> impl Future<Output = Result<Vec<Event>, color_eyre::Report>>;
}
