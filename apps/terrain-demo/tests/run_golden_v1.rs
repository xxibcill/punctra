//! Process-independent visibility checks for the owner-local v1 Run corpus.

#[test]
fn checked_run_v1_corpus_packages_all_boundaries_and_real_artifacts() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/run-v1/manifest.json"))
            .expect("checked Run fixture manifest is valid JSON");
    assert_eq!(manifest["checkpoint_count"], 8);
    assert_eq!(manifest["disk_version"], 1);
    assert_eq!(manifest["semantic_version"], 1);
    assert_eq!(manifest["frame_version"], 1);
    assert_eq!(manifest["files"].as_array().unwrap().len(), 13);

    let complete = include_bytes!("fixtures/run-v1/complete/run.pwf");
    let prefixes: [&[u8]; 8] = [
        include_bytes!("fixtures/run-v1/prefixes/01-intent.pwf"),
        include_bytes!("fixtures/run-v1/prefixes/02-revision-resolved.pwf"),
        include_bytes!("fixtures/run-v1/prefixes/03-audit-observed.pwf"),
        include_bytes!("fixtures/run-v1/prefixes/04-surface-observed.pwf"),
        include_bytes!("fixtures/run-v1/prefixes/05-qa-observed.pwf"),
        include_bytes!("fixtures/run-v1/prefixes/06-export-ensured.pwf"),
        include_bytes!("fixtures/run-v1/prefixes/07-report-ensured.pwf"),
        include_bytes!("fixtures/run-v1/prefixes/08-complete.pwf"),
    ];
    assert!(
        prefixes
            .windows(2)
            .all(|pair| pair[0].len() < pair[1].len())
    );
    assert_eq!(complete, prefixes[7]);

    let audit_bytes = include_bytes!("fixtures/run-v1/complete/audit.json");
    let audit: serde_json::Value =
        serde_json::from_slice(audit_bytes).expect("checked Run report fixture is JSON");
    assert_eq!(audit["schema"], "punctra.terrain-workflow.audit.v1");
    for (manifest_name, report_name) in [
        ("run_id", "run"),
        ("operation_id", "operation"),
        ("source_id", "source"),
        ("workspace_id", "workspace"),
        ("baseline_revision", "baseline_revision"),
        ("committed_revision", "changed_revision"),
    ] {
        assert_eq!(manifest[manifest_name], audit["identities"][report_name]);
    }
    assert_eq!(audit["terrain"]["baseline"]["input_point_count"], 64);
    assert_eq!(audit["terrain"]["changed"]["input_point_count"], 62);
    assert_eq!(
        audit["external_evidence"]["partner_acceptance_evaluated"],
        false
    );
    assert_eq!(
        audit["external_evidence"]["downstream_round_trip_evaluated"],
        false
    );
    assert_eq!(
        audit["external_evidence"]["human_workflow_acceptance_evaluated"],
        false
    );
    assert_no_absolute_path_values(&audit);

    let terrain = std::str::from_utf8(include_bytes!("fixtures/run-v1/complete/terrain.xml"))
        .expect("checked terrain artifact is UTF-8");
    let document = roxmltree::Document::parse(terrain).expect("checked terrain artifact is XML");
    let root = document.root_element();
    assert_eq!(root.tag_name().name(), "LandXML");
    assert_eq!(
        root.tag_name().namespace(),
        Some("http://www.landxml.org/schema/LandXML-1.2")
    );
    assert_eq!(
        document
            .descendants()
            .filter(|node| node.has_tag_name(("http://www.landxml.org/schema/LandXML-1.2", "P")))
            .count(),
        62
    );
    assert_eq!(
        document
            .descendants()
            .filter(|node| node.has_tag_name(("http://www.landxml.org/schema/LandXML-1.2", "F")))
            .count(),
        94
    );
}

fn assert_no_absolute_path_values(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            assert!(
                !text.starts_with('/'),
                "report exposes an absolute POSIX path"
            );
            assert!(
                !(text.len() >= 3
                    && text.as_bytes()[1] == b':'
                    && matches!(text.as_bytes()[2], b'\\' | b'/')),
                "report exposes an absolute drive path"
            );
            assert!(
                !text.starts_with("file:"),
                "report exposes an absolute file URI"
            );
        }
        serde_json::Value::Array(values) => {
            for value in values {
                assert_no_absolute_path_values(value);
            }
        }
        serde_json::Value::Object(fields) => {
            for value in fields.values() {
                assert_no_absolute_path_values(value);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}
