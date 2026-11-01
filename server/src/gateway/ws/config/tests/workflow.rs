use super::super::*;
use crate::config::Config;
use serde_json::json;

#[test]
fn workflow_partial_patch_replaces_bindings_atomically() {
    let mut base = Config::default();
    base.workflow = json!({"version":1,"bindings":[{"id":"old","plugin_id":"wakeup.greeting","position_id":"start"}]});
    let patch = json!({"workflow":{"version":1,"bindings":[{"id":"new","plugin_id":"soul.profile","position_id":"llm"}]}});
    let merged = merge_client_patch(&base, &patch).unwrap();
    assert_eq!(merged.workflow["bindings"].as_array().unwrap().len(), 1);
    assert_eq!(merged.workflow["bindings"][0]["id"], "new");
}
