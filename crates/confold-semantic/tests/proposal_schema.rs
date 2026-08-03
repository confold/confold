use confold_semantic::{prepare, proposal_json_schema, InputRole};
use std::fs;

#[test]
fn proposal_schema_binds_the_operation_and_available_input_roles() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let left = directory.path().join("left.md");
    let right = directory.path().join("right.md");
    fs::write(&left, "# Left\n").expect("write left");
    fs::write(&right, "# Right\n").expect("write right");
    let bundle = prepare(&left, &right, None).expect("prepare bundle");

    let schema = proposal_json_schema(&bundle);

    assert_eq!(
        schema["properties"]["operation_id"]["const"],
        bundle.operation_id
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["contributions"]["items"]["additionalProperties"],
        false
    );
    assert_eq!(
        schema["properties"]["contributions"]["items"]["properties"]["source"]["enum"],
        serde_json::json!(["left", "right"])
    );
    assert_eq!(
        schema["allOf"][0]["if"]["properties"]["verdict"]["const"],
        "merged"
    );
    assert_eq!(
        schema["allOf"][0]["then"]["properties"]["result"]["type"],
        "string"
    );
    assert_eq!(
        schema["allOf"][1]["if"]["properties"]["verdict"]["enum"],
        serde_json::json!(["equivalent", "prefer_left", "prefer_right", "uncertain"])
    );
    assert_eq!(
        schema["allOf"][1]["then"]["properties"]["result"]["type"],
        "null"
    );
    assert_eq!(
        bundle
            .inputs
            .iter()
            .map(|input| input.role)
            .collect::<Vec<_>>(),
        vec![InputRole::Left, InputRole::Right]
    );
}
