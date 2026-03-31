use anvil_tools::all_tool_definitions;

#[test]
fn all_tools_have_required_fields() {
    let defs = all_tool_definitions();
    assert_eq!(defs.len(), 11, "should have exactly 11 tools");

    let expected_names = [
        "file_read",
        "file_write",
        "file_edit",
        "shell",
        "grep",
        "ls",
        "find",
        "git_status",
        "git_diff",
        "git_log",
        "git_commit",
    ];

    for (def, expected_name) in defs.iter().zip(expected_names.iter()) {
        assert_eq!(def["type"], "function");
        let func = &def["function"];
        assert_eq!(func["name"].as_str().unwrap(), *expected_name);
        assert!(!func["description"].as_str().unwrap().is_empty());
        assert!(func["parameters"]["type"] == "object");
        assert!(func["parameters"]["properties"].is_object());
    }
}

#[test]
fn tool_definitions_are_valid_json_schema() {
    let defs = all_tool_definitions();
    for def in &defs {
        let params = &def["function"]["parameters"];
        assert_eq!(params["type"], "object");
        assert!(params["properties"].is_object());
        for (_, prop) in params["properties"].as_object().unwrap() {
            assert!(prop["type"].is_string());
            assert!(prop["description"].is_string());
        }
    }
}
