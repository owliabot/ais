use super::super::ref_model::RefPath;

#[test]
fn parse_ref_path_input_variants() {
    assert_eq!(
        RefPath::parse("inputs.owner"),
        Some(RefPath::Input {
            slot: "owner".to_string()
        })
    );
    assert_eq!(
        RefPath::parse("runtime.inputs.token.address"),
        Some(RefPath::Input {
            slot: "token.address".to_string()
        })
    );
    assert_eq!(
        RefPath::parse("token.decimals"),
        Some(RefPath::Input {
            slot: "token.decimals".to_string()
        })
    );
    assert_eq!(
        RefPath::parse("missing_ref=inputs.owner"),
        Some(RefPath::Input {
            slot: "owner".to_string()
        })
    );
}

#[test]
fn parse_ref_path_fact_variants() {
    assert_eq!(
        RefPath::parse("facts.quote.slippage_bps"),
        Some(RefPath::Fact {
            key: "quote.slippage_bps".to_string()
        })
    );
    assert_eq!(
        RefPath::parse("fact:token.decimals"),
        Some(RefPath::Fact {
            key: "token.decimals".to_string()
        })
    );
    assert_eq!(
        RefPath::parse("runtime.facts.transfer.completed"),
        Some(RefPath::Fact {
            key: "transfer.completed".to_string()
        })
    );
}

#[test]
fn parse_ref_path_node_output_variants() {
    assert_eq!(
        RefPath::parse("nodes.seg_1__q_balance.outputs.balance"),
        Some(RefPath::NodeOutput {
            step_id: "seg_1__q_balance".to_string(),
            field_path: "balance".to_string()
        })
    );
    assert_eq!(
        RefPath::parse("nodes[\"seg_1__q_balance\"].outputs.balance"),
        Some(RefPath::NodeOutput {
            step_id: "seg_1__q_balance".to_string(),
            field_path: "balance".to_string()
        })
    );
    assert_eq!(
        RefPath::parse("nodes['seg_1__q_balance'].output.balance"),
        Some(RefPath::NodeOutput {
            step_id: "seg_1__q_balance".to_string(),
            field_path: "balance".to_string()
        })
    );
}

#[test]
fn parse_ref_path_rejects_non_bindable_roots() {
    assert_eq!(RefPath::parse("params.token.address"), None);
    assert_eq!(RefPath::parse("runtime.params.token.address"), None);
    assert_eq!(RefPath::parse("workspace.state"), None);
}

#[test]
fn parse_ref_path_canonical_display() {
    assert_eq!(
        RefPath::Input {
            slot: "token.decimals".to_string()
        }
        .as_canonical_str(),
        "inputs.token.decimals"
    );
    assert_eq!(
        RefPath::Fact {
            key: "quote.slippage_bps".to_string()
        }
        .to_string(),
        "facts.quote.slippage_bps"
    );
    assert_eq!(
        RefPath::NodeOutput {
            step_id: "seg_1__q_balance".to_string(),
            field_path: "balance".to_string()
        }
        .as_canonical_str(),
        "nodes.seg_1__q_balance.outputs.balance"
    );
}
