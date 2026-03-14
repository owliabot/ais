use ais_agent_control::execution_artifact::TransactionStage;

use crate::{runtime::ActiveRun, runtime_value_resolver::resolve_value_ref};

pub(crate) fn export_transaction_outputs(
    runtime: &mut ActiveRun,
    stage: &TransactionStage,
) -> Result<Vec<String>, String> {
    if runtime.checkpoint.execution_artifact.is_none() {
        return Err("execution_artifact runtime state is not present".to_owned());
    }

    let mut resolved = Vec::new();
    for export in &stage.exports {
        resolved.push((
            export.output_key.clone(),
            resolve_value_ref(runtime, &export.source)?,
        ));
    }

    let snapshot = runtime
        .checkpoint
        .execution_artifact
        .as_mut()
        .expect("checked is_some");
    let mut exported = Vec::new();
    for (output_key, value) in resolved {
        snapshot.exported_outputs.insert(output_key.clone(), value);
        exported.push(output_key.to_string());
    }
    Ok(exported)
}
