use alloy::{
    dyn_abi::{EventExt, JsonAbiExt},
    primitives::Selector,
    sol_types::SolCall,
};
use color_eyre::eyre::eyre;
use revm_inspectors::tracing::{
    CallTraceArena, TraceWriter,
    types::{CallTrace, DecodedCallData, DecodedCallLog, DecodedCallTrace},
};

use crate::{
    fmt::dynamic::format_token,
    fourbyte::FourBytesProvider,
    openchain::OpenChainClient,
    trace_utils::{get_indexed_event, reconstruct_params},
};

alloy::sol! {
    contract Revert {
        function Error(string s) external;
    }
}

pub fn decode_revert(bs: &[u8]) -> Result<String, color_eyre::Report> {
    Ok(Revert::ErrorCall::abi_decode(bs)?.s)
}

fn decode_return_data(trace: &CallTrace) -> Option<String> {
    (!trace.success)
        .then(|| decode_revert(&trace.output).unwrap_or(alloy::hex::encode(&trace.output)))
}

pub async fn decode_calltrace<P: FourBytesProvider>(
    calls: &mut CallTraceArena,
    client: P,
) -> Result<(), color_eyre::Report> {
    for node in calls.nodes_mut() {
        if node.trace.data.len() > 4 {
            let selector = Selector::from_slice(&node.trace.data[0..4]);
            let funcs = client.function_signatures(selector).await?;
            node.trace.decoded = Some(Box::new(DecodedCallTrace {
                label: None,
                call_data: None,
                return_data: decode_return_data(&node.trace),
            }));
            if !funcs.is_empty() {
                for func in funcs {
                    if let Ok(args) = func.abi_decode_input(&node.trace.data[4..]) {
                        tracing::debug!("Function decoded with: {:?}", &func);
                        node.trace.decoded = Some(Box::new(DecodedCallTrace {
                            label: None,
                            call_data: Some(DecodedCallData {
                                signature: func.signature(),
                                args: args.into_iter().map(|arg| format_token(&arg)).collect(),
                            }),
                            return_data: decode_return_data(&node.trace),
                        }));
                        break;
                    }
                }
            }
        }

        for log in node.logs.iter_mut() {
            let decoded = if !log.raw_log.topics().is_empty() {
                let t0 = &log.raw_log.topics()[0];

                let events = client.event_signatures(*t0).await?;

                if !events.is_empty() {
                    let mut ret = DecodedCallLog::default();
                    for event in events {
                        let event = get_indexed_event(event, &log.raw_log);
                        // debug!("Log: {:?}, event: {:?}", &log.raw_log, &event);
                        if let Ok(decoded) = event.decode_log(&log.raw_log) {
                            tracing::debug!("Event decoded with: {:?}", &event);
                            let params = reconstruct_params(&event, &decoded);
                            ret.name = Some(event.name.clone());
                            ret.params = Some(
                                params
                                    .into_iter()
                                    .zip(event.inputs.iter())
                                    .map(|(param, input)| {
                                        // undo patched names
                                        let name = input.name.clone();
                                        (name, format_token(&param))
                                    })
                                    .collect(),
                            );
                            break;
                        }
                    }
                    ret
                } else {
                    DecodedCallLog::default()
                }
            } else {
                DecodedCallLog::default()
            };
            log.decoded = Some(Box::new(decoded));
        }
    }

    Ok(())
}

pub async fn decode_calltrace_print(
    mut trace: CallTraceArena,
) -> Result<String, color_eyre::Report> {
    let client = OpenChainClient::new();
    decode_calltrace(&mut trace, client).await?;
    let mut out = vec![];
    let mut writer = TraceWriter::new(&mut out);
    writer.write_arena(&trace).unwrap();
    String::from_utf8(out).map_err(|_| eyre!("trace has non-utf8"))
}
