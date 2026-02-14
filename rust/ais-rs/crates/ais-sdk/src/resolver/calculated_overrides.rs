use crate::resolver::ValueRef;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalculatedOverrideError {
    OverridesMustBeObject,
    EntryMustBeObject {
        key: String,
    },
    ExprMissing {
        key: String,
    },
    ExprInvalid {
        key: String,
        reason: String,
    },
    MissingDependency {
        key: String,
        dependency: String,
    },
    DependencyCycle {
        cycle: Vec<String>,
    },
}

pub fn calculated_override_order(
    overrides: &Value,
) -> Result<Vec<String>, Vec<CalculatedOverrideError>> {
    let Some(overrides_obj) = overrides.as_object() else {
        return Err(vec![CalculatedOverrideError::OverridesMustBeObject]);
    };
    calculated_override_order_from_map(overrides_obj)
}

pub fn calculated_override_order_from_map(
    overrides_obj: &Map<String, Value>,
) -> Result<Vec<String>, Vec<CalculatedOverrideError>> {
    let mut dependencies = HashMap::<String, BTreeSet<String>>::new();
    let mut errors = Vec::<CalculatedOverrideError>::new();

    for (key, entry) in overrides_obj {
        let Some(entry_obj) = entry.as_object() else {
            errors.push(CalculatedOverrideError::EntryMustBeObject { key: key.clone() });
            continue;
        };
        let Some(expr) = entry_obj.get("expr") else {
            errors.push(CalculatedOverrideError::ExprMissing { key: key.clone() });
            continue;
        };
        let value_ref = match serde_json::from_value::<ValueRef>(expr.clone()) {
            Ok(value_ref) => value_ref,
            Err(error) => {
                errors.push(CalculatedOverrideError::ExprInvalid {
                    key: key.clone(),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        dependencies.insert(key.clone(), collect_calculated_dependencies(&value_ref));
    }

    let override_keys = overrides_obj.keys().cloned().collect::<BTreeSet<_>>();
    for (key, deps) in &dependencies {
        for dep in deps {
            if !override_keys.contains(dep) {
                errors.push(CalculatedOverrideError::MissingDependency {
                    key: key.clone(),
                    dependency: dep.clone(),
                });
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let mut indegree = HashMap::<String, usize>::new();
    let mut reverse = HashMap::<String, Vec<String>>::new();
    for key in &override_keys {
        indegree.insert(key.clone(), 0);
    }
    for (key, deps) in &dependencies {
        indegree.insert(key.clone(), deps.len());
        for dep in deps {
            reverse.entry(dep.clone()).or_default().push(key.clone());
        }
    }
    for children in reverse.values_mut() {
        children.sort();
    }

    let mut ready = indegree
        .iter()
        .filter_map(|(key, degree)| if *degree == 0 { Some(key.clone()) } else { None })
        .collect::<Vec<_>>();
    ready.sort();
    let mut queue = VecDeque::from(ready);
    let mut order = Vec::<String>::new();
    while let Some(key) = queue.pop_front() {
        order.push(key.clone());
        for child in reverse.get(&key).cloned().unwrap_or_default() {
            let entry = indegree.entry(child.clone()).or_insert(0);
            if *entry > 0 {
                *entry -= 1;
                if *entry == 0 {
                    queue.push_back(child);
                }
            }
        }
        let mut sorted = queue.into_iter().collect::<Vec<_>>();
        sorted.sort();
        queue = VecDeque::from(sorted);
    }

    if order.len() != override_keys.len() {
        let mut unresolved = indegree
            .into_iter()
            .filter_map(|(key, degree)| if degree > 0 { Some(key) } else { None })
            .collect::<Vec<_>>();
        unresolved.sort();
        return Err(vec![CalculatedOverrideError::DependencyCycle { cycle: unresolved }]);
    }

    Ok(order)
}

fn collect_calculated_dependencies(value_ref: &ValueRef) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_calculated_dependencies_inner(value_ref, &mut out);
    out
}

fn collect_calculated_dependencies_inner(value_ref: &ValueRef, out: &mut BTreeSet<String>) {
    match value_ref {
        ValueRef::Lit { .. } => {}
        ValueRef::Ref { ref_path } => {
            if let Some(dep) = extract_calculated_from_ref(ref_path.as_str()) {
                out.insert(dep);
            }
        }
        ValueRef::Cel { cel } => {
            for dep in extract_calculated_from_cel(cel.as_str()) {
                out.insert(dep);
            }
        }
        ValueRef::Detect { .. } => {}
        ValueRef::Object { object } => {
            for child in object.values() {
                collect_calculated_dependencies_inner(child, out);
            }
        }
        ValueRef::Array { array } => {
            for child in array {
                collect_calculated_dependencies_inner(child, out);
            }
        }
    }
}

fn extract_calculated_from_ref(path: &str) -> Option<String> {
    let normalized = path.trim().trim_start_matches('$').trim_start_matches('.');
    let mut iter = normalized.split('.');
    match (iter.next(), iter.next()) {
        (Some("calculated"), Some(name)) if !name.trim().is_empty() => Some(name.to_string()),
        _ => None,
    }
}

fn extract_calculated_from_cel(cel: &str) -> Vec<String> {
    let regex = Regex::new(r"\bcalculated\.([A-Za-z_][A-Za-z0-9_-]*)\b").expect("valid regex");
    regex
        .captures_iter(cel)
        .filter_map(|capture| capture.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

#[cfg(test)]
#[path = "calculated_overrides_test.rs"]
mod tests;
