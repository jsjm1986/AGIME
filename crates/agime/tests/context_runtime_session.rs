use agime::context_runtime::{
    project_for_provider, CollapseCommit, CompactBoundaryMetadata, CompactDirection,
    ContextRuntimeState, PreservedSegmentMetadata,
};
use agime::conversation::message::Message;
use agime::conversation::Conversation;
use agime::session::extension_data::ExtensionData;
use agime::session::{ExtensionState, SessionManager, SessionType};
use serde_json::json;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn context_runtime_state_survives_export_import_and_clears_on_truncate() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-export".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let state = ContextRuntimeState {
        runtime_compactions: 2,
        ..ContextRuntimeState::default()
    };
    let mut state = state;
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 1,
        head_index: 1,
        tail_index: 2,
        direction: CompactDirection::UpTo,
        anchor_message_id: None,
        head_message_id: None,
        tail_message_id: None,
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 2,
        end_index: 2,
        tail_anchor_index: 2,
        start_message_id: None,
        end_message_id: None,
        tail_anchor_message_id: None,
        reason: "test_boundary".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let mut first = Message::user().with_text("one");
    first.created = 10;
    let mut second = Message::assistant().with_text("two");
    second.created = 20;
    let mut third = Message::user().with_text("three");
    third.created = 30;
    let conversation = Conversation::new_unvalidated(vec![first, second, third]);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    let exported = SessionManager::export_session(&session.id)
        .await
        .expect("export session");
    let imported = SessionManager::import_session(&exported)
        .await
        .expect("import session");

    assert!(
        ContextRuntimeState::from_extension_data(&imported.extension_data).is_none(),
        "cached-only runtime state should not survive export/import without structural entries"
    );

    SessionManager::truncate_conversation(&imported.id, 20)
        .await
        .expect("truncate conversation");
    let truncated = SessionManager::get_session(&imported.id, true)
        .await
        .expect("load truncated session");

    assert!(
        ContextRuntimeState::from_extension_data(&truncated.extension_data).is_none(),
        "truncate should clear context runtime state"
    );
    assert_eq!(
        truncated
            .conversation
            .expect("conversation")
            .messages()
            .len(),
        1
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete original session");
    SessionManager::delete_session(&imported.id)
        .await
        .expect("delete imported session");
}

#[tokio::test]
#[serial]
async fn legacy_live_store_is_backfilled_on_session_roundtrip() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-legacy-roundtrip".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let state = ContextRuntimeState {
        schema_version: 5,
        store: agime::context_runtime::ContextRuntimeStore {
            entry_log: Vec::new(),
            collapse_commits: vec![CollapseCommit {
                commit_id: Some("legacy-commit".to_string()),
                summary: "legacy".to_string(),
                start_index: 1,
                end_index: 3,
                direction: CompactDirection::UpTo,
                start_message_id: None,
                end_message_id: None,
                created_at: 1,
            }],
            ..Default::default()
        },
        ..ContextRuntimeState::default()
    };

    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist legacy state");

    let loaded = SessionManager::get_session(&session.id, false)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data)
        .expect("load context runtime state");

    assert_eq!(
        loaded_state.schema_version,
        agime::context_runtime::CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION
    );
    assert_eq!(loaded_state.store.entry_log.len(), 1);
    assert_eq!(loaded_state.committed_collapses().len(), 1);
    assert_eq!(loaded_state.committed_collapses()[0].summary, "legacy");

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn mixed_format_context_runtime_extension_data_is_canonicalized_on_load() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-mixed-load".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut extension_data = ExtensionData::new();
    extension_data.set_extension_state(
        "context_runtime",
        "v0",
        json!({
            "schemaVersion": 5,
            "store": {
                "entryLog": [
                    { "kind": "snipEntriesCleared" },
                    { "kind": "microcompactEntriesCleared" }
                ],
                "snipEntries": [],
                "microcompactEntries": []
            },
            "committedCollapses": [
                {
                    "commitId": "legacy-commit",
                    "summary": "legacy",
                    "startIndex": 1,
                    "endIndex": 3,
                    "direction": "up_to",
                    "createdAt": 1
                }
            ],
            "snipRemovals": [
                {
                    "removedIndexes": [1],
                    "removedMessageIds": ["m2"],
                    "removedCount": 1,
                    "tokenEstimateFreed": 50,
                    "reason": "legacy",
                    "createdAt": 1
                }
            ]
        }),
    );

    SessionManager::update_session(&session.id)
        .extension_data(extension_data)
        .apply()
        .await
        .expect("persist raw extension data");

    let loaded = SessionManager::get_session(&session.id, false)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data)
        .expect("load context runtime state");

    assert_eq!(
        loaded_state.schema_version,
        agime::context_runtime::CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION
    );
    assert_eq!(loaded_state.committed_collapses().len(), 1);
    assert_eq!(loaded_state.committed_collapses()[0].summary, "legacy");
    assert!(loaded_state.snip_entries().is_empty());
    assert!(loaded_state.microcompact_entries().is_empty());
    assert!(loaded_state.store.entry_log.len() >= 3);

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn update_session_canonicalizes_context_runtime_extension_data_at_write_boundary() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-update-boundary".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut extension_data = ExtensionData::new();
    extension_data.set_extension_state(
        "context_runtime",
        "v0",
        json!({
            "schemaVersion": 5,
            "store": {
                "entryLog": [
                    { "kind": "snipEntriesCleared" },
                    { "kind": "microcompactEntriesCleared" }
                ],
                "snipEntries": [],
                "microcompactEntries": []
            },
            "committedCollapses": [
                {
                    "commitId": "legacy-commit",
                    "summary": "legacy",
                    "startIndex": 1,
                    "endIndex": 3,
                    "direction": "up_to",
                    "createdAt": 1
                }
            ]
        }),
    );

    SessionManager::update_session(&session.id)
        .extension_data(extension_data)
        .apply()
        .await
        .expect("persist raw extension data");

    let loaded = SessionManager::get_session(&session.id, false)
        .await
        .expect("load session");
    let runtime = loaded
        .extension_data
        .get_extension_state("context_runtime", "v0")
        .expect("raw runtime state");

    assert_eq!(
        runtime
            .get("schemaVersion")
            .and_then(|value| value.as_u64()),
        Some(agime::context_runtime::CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION as u64)
    );
    assert!(runtime
        .get("store")
        .and_then(|store| store.get("entryLog"))
        .and_then(|entry_log| entry_log.as_array())
        .is_some_and(|entry_log| entry_log.len() >= 3));
    assert!(runtime
        .get("store")
        .and_then(|store| store.get("snipEntries"))
        .and_then(|entries| entries.as_array())
        .is_some_and(|entries| entries.is_empty()));

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn mixed_format_context_runtime_state_persists_back_as_canonical_wire() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-mixed-persist".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut extension_data = ExtensionData::new();
    extension_data.set_extension_state(
        "context_runtime",
        "v0",
        json!({
            "schemaVersion": 5,
            "store": {
                "entryLog": [
                    { "kind": "snipEntriesCleared" },
                    { "kind": "microcompactEntriesCleared" }
                ],
                "snipEntries": [],
                "microcompactEntries": []
            },
            "committedCollapses": [
                {
                    "commitId": "legacy-commit",
                    "summary": "legacy",
                    "startIndex": 1,
                    "endIndex": 3,
                    "direction": "up_to",
                    "createdAt": 1
                }
            ],
            "snipRemovals": [
                {
                    "removedIndexes": [1],
                    "removedMessageIds": ["m2"],
                    "removedCount": 1,
                    "tokenEstimateFreed": 50,
                    "reason": "legacy",
                    "createdAt": 1
                }
            ]
        }),
    );

    SessionManager::update_session(&session.id)
        .extension_data(extension_data)
        .apply()
        .await
        .expect("persist raw extension data");

    let loaded = SessionManager::get_session(&session.id, false)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data)
        .expect("load context runtime state");

    SessionManager::set_extension_state(&session.id, &loaded_state)
        .await
        .expect("persist canonicalized state");

    let persisted = SessionManager::get_session(&session.id, false)
        .await
        .expect("reload session");
    let raw_state = persisted
        .extension_data
        .get_extension_state("context_runtime", "v0")
        .expect("raw runtime state");

    assert_eq!(
        raw_state
            .get("schemaVersion")
            .and_then(|value| value.as_u64()),
        Some(agime::context_runtime::CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION as u64)
    );
    assert!(raw_state
        .get("store")
        .and_then(|store| store.get("entryLog"))
        .and_then(|entry_log| entry_log.as_array())
        .is_some_and(|entry_log| entry_log.len() >= 3));
    assert!(raw_state
        .get("store")
        .and_then(|store| store.get("collapseCommits"))
        .and_then(|commits| commits.as_array())
        .is_some_and(|commits| commits.len() == 1));
    assert!(raw_state
        .get("store")
        .and_then(|store| store.get("snipEntries"))
        .and_then(|entries| entries.as_array())
        .is_some_and(|entries| entries.is_empty()));

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn export_session_canonicalizes_mixed_format_context_runtime_wire() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-export-canonical".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut extension_data = ExtensionData::new();
    extension_data.set_extension_state(
        "context_runtime",
        "v0",
        json!({
            "schemaVersion": 5,
            "store": {
                "entryLog": [
                    { "kind": "snipEntriesCleared" },
                    { "kind": "microcompactEntriesCleared" }
                ],
                "snipEntries": [],
                "microcompactEntries": []
            },
            "committedCollapses": [
                {
                    "commitId": "legacy-commit",
                    "summary": "legacy",
                    "startIndex": 1,
                    "endIndex": 3,
                    "direction": "up_to",
                    "createdAt": 1
                }
            ],
            "snipRemovals": [
                {
                    "removedIndexes": [1],
                    "removedMessageIds": ["m2"],
                    "removedCount": 1,
                    "tokenEstimateFreed": 50,
                    "reason": "legacy",
                    "createdAt": 1
                }
            ]
        }),
    );

    SessionManager::update_session(&session.id)
        .extension_data(extension_data)
        .apply()
        .await
        .expect("persist raw extension data");

    let exported = SessionManager::export_session(&session.id)
        .await
        .expect("export session");
    let exported_json: serde_json::Value = serde_json::from_str(&exported).expect("export json");
    let runtime = exported_json
        .get("extension_data")
        .and_then(|ext| ext.get("context_runtime.v0"))
        .expect("exported runtime state");

    assert_eq!(
        runtime
            .get("schemaVersion")
            .and_then(|value| value.as_u64()),
        Some(agime::context_runtime::CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION as u64)
    );
    assert!(runtime
        .get("store")
        .and_then(|store| store.get("entryLog"))
        .and_then(|entry_log| entry_log.as_array())
        .is_some_and(|entry_log| entry_log.len() >= 3));
    assert!(runtime
        .get("store")
        .and_then(|store| store.get("snipEntries"))
        .and_then(|entries| entries.as_array())
        .is_some_and(|entries| entries.is_empty()));

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn copy_session_canonicalizes_context_runtime_extension_data() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-copy-canonical".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut extension_data = ExtensionData::new();
    extension_data.set_extension_state(
        "context_runtime",
        "v0",
        json!({
            "schemaVersion": 5,
            "store": {
                "entryLog": [
                    { "kind": "snipEntriesCleared" }
                ],
                "snipEntries": []
            },
            "committedCollapses": [
                {
                    "commitId": "legacy-commit",
                    "summary": "legacy",
                    "startIndex": 1,
                    "endIndex": 3,
                    "direction": "up_to",
                    "createdAt": 1
                }
            ]
        }),
    );

    SessionManager::update_session(&session.id)
        .extension_data(extension_data)
        .apply()
        .await
        .expect("persist raw extension data");

    let copied = SessionManager::copy_session(&session.id, "copied".to_string())
        .await
        .expect("copy session");

    let runtime = copied
        .extension_data
        .get_extension_state("context_runtime", "v0")
        .expect("copied runtime state");

    assert_eq!(
        runtime
            .get("schemaVersion")
            .and_then(|value| value.as_u64()),
        Some(agime::context_runtime::CURRENT_CONTEXT_RUNTIME_SCHEMA_VERSION as u64)
    );
    assert!(runtime
        .get("store")
        .and_then(|store| store.get("entryLog"))
        .and_then(|entry_log| entry_log.as_array())
        .is_some_and(|entry_log| entry_log.len() >= 2));

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete source session");
    SessionManager::delete_session(&copied.id)
        .await
        .expect("delete copied session");
}

#[tokio::test]
#[serial]
async fn committed_collapse_survives_export_import_projection() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-commit-export".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut state = ContextRuntimeState {
        runtime_compactions: 1,
        ..ContextRuntimeState::default()
    };
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-1".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 1,
        head_index: 1,
        tail_index: 3,
        direction: CompactDirection::UpTo,
        anchor_message_id: None,
        head_message_id: None,
        tail_message_id: None,
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 3,
        end_index: 4,
        tail_anchor_index: 4,
        start_message_id: None,
        end_message_id: None,
        tail_anchor_message_id: None,
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    let exported = SessionManager::export_session(&session.id)
        .await
        .expect("export session");
    let imported = SessionManager::import_session(&exported)
        .await
        .expect("import session");

    let imported_state = ContextRuntimeState::from_extension_data(&imported.extension_data)
        .expect("imported runtime state");
    let imported_conversation = imported.conversation.expect("conversation");
    let projected = project_for_provider(&imported_conversation, &imported_state);
    let texts = projected
        .messages()
        .iter()
        .map(|message| message.as_concat_text())
        .collect::<Vec<_>>();

    assert!(texts.iter().any(|text| text.contains("collapsed summary")));
    assert!(texts.iter().any(|text| text == "kept-1"));
    assert!(texts.iter().any(|text| text == "kept-2"));

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete original session");
    SessionManager::delete_session(&imported.id)
        .await
        .expect("delete imported session");
}

#[tokio::test]
#[serial]
async fn truncate_conversation_preserves_valid_committed_collapse_state() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-truncate-commit".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut state = ContextRuntimeState {
        runtime_compactions: 1,
        ..ContextRuntimeState::default()
    };
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-1".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 1,
        head_index: 1,
        tail_index: 3,
        direction: CompactDirection::UpTo,
        anchor_message_id: None,
        head_message_id: None,
        tail_message_id: None,
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 3,
        end_index: 4,
        tail_anchor_index: 4,
        start_message_id: None,
        end_message_id: None,
        tail_anchor_message_id: None,
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let mut messages = vec![
        Message::user().with_text("m1"),
        Message::assistant().with_text("m2"),
        Message::user().with_text("m3"),
        Message::assistant().with_text("m4"),
        Message::user().with_text("m5"),
    ];
    for (index, message) in messages.iter_mut().enumerate() {
        message.created = ((index + 1) * 10) as i64;
    }
    let conversation = Conversation::new_unvalidated(messages);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    SessionManager::truncate_conversation(&session.id, 50)
        .await
        .expect("truncate conversation");
    let truncated = SessionManager::get_session(&session.id, true)
        .await
        .expect("load truncated session");
    let state = ContextRuntimeState::from_extension_data(&truncated.extension_data)
        .expect("runtime state should survive");

    assert!(state.staged_snapshot().is_none());
    assert_eq!(state.committed_collapses().len(), 1);
    assert_eq!(state.compact_boundary().expect("boundary").tail_index, 3);
    assert_eq!(
        state
            .preserved_segment()
            .expect("preserved segment")
            .start_index,
        3
    );
    assert_eq!(
        state
            .preserved_segment()
            .expect("preserved segment")
            .end_index,
        3
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn truncate_conversation_clears_idless_session_memory_when_suffix_removed() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-truncate-memory-clear".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut messages = vec![
        Message::user().with_text("m1"),
        Message::assistant().with_text("m2"),
        Message::user().with_text("m3"),
        Message::assistant().with_text("m4"),
        Message::user().with_text("m5"),
    ];
    for (index, message) in messages.iter_mut().enumerate() {
        message.created = ((index + 1) * 10) as i64;
    }
    let conversation = Conversation::new_unvalidated(messages);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    let mut state = ContextRuntimeState::default();
    state.set_session_memory(Some(agime::context_runtime::SessionMemoryState {
        summary: "memory summary".to_string(),
        summarized_through_message_id: None,
        preserved_start_index: 3,
        preserved_end_index: 4,
        preserved_start_message_id: None,
        preserved_end_message_id: None,
        preserved_message_count: 2,
        preserved_token_estimate: 10,
        tail_anchor_index: 4,
        tail_anchor_message_id: None,
        updated_at: 1,
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    SessionManager::truncate_conversation(&session.id, 40)
        .await
        .expect("truncate conversation");

    let truncated = SessionManager::get_session(&session.id, false)
        .await
        .expect("load truncated session");
    let loaded_state = ContextRuntimeState::from_extension_data(&truncated.extension_data);
    assert!(
        loaded_state
            .as_ref()
            .is_none_or(|state| state.session_memory().is_none()),
        "session memory should clear when preserved suffix is removed by truncation"
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn truncate_conversation_clears_idless_from_direction_runtime_state_when_tail_removed() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-truncate-from-clear".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut messages = vec![
        Message::user().with_text("keep-1"),
        Message::assistant().with_text("keep-2"),
        Message::user().with_text("keep-3"),
        Message::assistant().with_text("tail-1"),
        Message::user().with_text("tail-2"),
    ];
    for (index, message) in messages.iter_mut().enumerate() {
        message.created = ((index + 1) * 10) as i64;
    }
    let conversation = Conversation::new_unvalidated(messages);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-from-truncate".to_string()),
        summary: "collapsed tail summary".to_string(),
        start_index: 3,
        end_index: 5,
        direction: CompactDirection::From,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 3,
        head_index: 2,
        tail_index: 4,
        direction: CompactDirection::From,
        anchor_message_id: None,
        head_message_id: None,
        tail_message_id: None,
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 0,
        end_index: 2,
        tail_anchor_index: 2,
        start_message_id: None,
        end_message_id: None,
        tail_anchor_message_id: None,
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    SessionManager::truncate_conversation(&session.id, 40)
        .await
        .expect("truncate conversation");

    let truncated = SessionManager::get_session(&session.id, false)
        .await
        .expect("load truncated session");
    let loaded_state = ContextRuntimeState::from_extension_data(&truncated.extension_data);
    assert!(
        loaded_state.as_ref().is_none_or(|state| {
            state.committed_collapses().is_empty()
                && state.session_memory().is_none()
                && state.staged_snapshot().is_none()
        }),
        "from-direction collapse should clear when the collapsed tail is removed by truncation"
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn committed_from_direction_survives_export_import_projection() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-from-export".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut state = ContextRuntimeState {
        runtime_compactions: 1,
        ..ContextRuntimeState::default()
    };
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-from".to_string()),
        summary: "collapsed tail summary".to_string(),
        start_index: 3,
        end_index: 5,
        direction: CompactDirection::From,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 3,
        head_index: 2,
        tail_index: 4,
        direction: CompactDirection::From,
        anchor_message_id: None,
        head_message_id: None,
        tail_message_id: None,
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 0,
        end_index: 2,
        tail_anchor_index: 2,
        start_message_id: None,
        end_message_id: None,
        tail_anchor_message_id: None,
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("keep-1"),
        Message::assistant().with_text("keep-2"),
        Message::user().with_text("keep-3"),
        Message::assistant().with_text("tail-1"),
        Message::user().with_text("tail-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    let exported = SessionManager::export_session(&session.id)
        .await
        .expect("export session");
    let imported = SessionManager::import_session(&exported)
        .await
        .expect("import session");

    let imported_state = ContextRuntimeState::from_extension_data(&imported.extension_data)
        .expect("imported runtime state");
    let imported_conversation = imported.conversation.expect("conversation");
    let projected = project_for_provider(&imported_conversation, &imported_state);
    let texts = projected
        .messages()
        .iter()
        .map(|message| message.as_concat_text())
        .collect::<Vec<_>>();

    assert!(texts.iter().any(|text| text == "keep-1"));
    assert!(texts.iter().any(|text| text == "keep-2"));
    assert!(texts
        .iter()
        .any(|text| text.contains("collapsed tail summary")));
    assert!(!texts.iter().any(|text| text == "tail-1"));
    assert!(!texts.iter().any(|text| text == "tail-2"));

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete original session");
    SessionManager::delete_session(&imported.id)
        .await
        .expect("delete imported session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_relinks_idless_from_direction_runtime_state_by_message_signature() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-from-relink".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("keep-1"),
        Message::assistant().with_text("keep-2"),
        Message::user().with_text("keep-3"),
        Message::assistant().with_text("tail-1"),
        Message::user().with_text("tail-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-from-relink".to_string()),
        summary: "collapsed tail summary".to_string(),
        start_index: 3,
        end_index: 5,
        direction: CompactDirection::From,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 3,
        head_index: 2,
        tail_index: 4,
        direction: CompactDirection::From,
        anchor_message_id: None,
        head_message_id: None,
        tail_message_id: None,
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 0,
        end_index: 2,
        tail_anchor_index: 2,
        start_message_id: None,
        end_message_id: None,
        tail_anchor_message_id: None,
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("keep-1"),
        Message::assistant().with_text("keep-2"),
        Message::user().with_text("keep-3"),
        Message::assistant().with_text("tail-1"),
        Message::user().with_text("tail-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state =
        ContextRuntimeState::from_extension_data(&loaded.extension_data).expect("runtime state");
    let projected =
        project_for_provider(&loaded.conversation.expect("conversation"), &loaded_state);
    let texts = projected
        .messages()
        .iter()
        .map(|message| message.as_concat_text())
        .collect::<Vec<_>>();

    assert!(texts.iter().any(|text| text == "keep-1"));
    assert!(texts.iter().any(|text| text == "keep-2"));
    assert!(texts.iter().any(|text| text == "keep-3"));
    assert!(texts
        .iter()
        .any(|text| text.contains("collapsed tail summary")));
    assert!(!texts.iter().any(|text| text == "tail-1"));
    assert!(!texts.iter().any(|text| text == "tail-2"));
    assert_eq!(
        loaded_state.compact_boundary().expect("boundary").direction,
        CompactDirection::From
    );
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .end_index,
        2
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_refreshes_context_runtime_projection_cache() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-replace-refresh".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-refresh".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.last_projection_stats = Some(agime::context_runtime::ProjectionStats {
        base_agent_messages: 999,
        projected_agent_messages: 999,
        snip_removed_count: 999,
        microcompacted_count: 999,
        raw_token_estimate: 9999,
        projected_token_estimate: 9999,
        freed_token_estimate: 0,
        updated_at: 1,
    });
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist runtime state");

    let conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data)
        .expect("load context runtime state");

    assert_eq!(
        loaded_state
            .compact_boundary()
            .expect("boundary")
            .tail_index,
        3
    );
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .start_index,
        3
    );
    assert_ne!(
        loaded_state
            .last_projection_stats
            .as_ref()
            .expect("projection stats")
            .raw_token_estimate,
        9999
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_relinks_idless_runtime_state_by_message_signature() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-relink-by-signature".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-relink".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 1,
        head_index: 1,
        tail_index: 3,
        direction: CompactDirection::UpTo,
        anchor_message_id: None,
        head_message_id: None,
        tail_message_id: None,
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 3,
        end_index: 4,
        tail_anchor_index: 4,
        start_message_id: None,
        end_message_id: None,
        tail_anchor_message_id: None,
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
        Message::assistant().with_text("new-tail"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state =
        ContextRuntimeState::from_extension_data(&loaded.extension_data).expect("runtime state");
    let projected =
        project_for_provider(&loaded.conversation.expect("conversation"), &loaded_state);
    let texts = projected
        .messages()
        .iter()
        .map(|message| message.as_concat_text())
        .collect::<Vec<_>>();

    assert!(texts.iter().any(|text| text.contains("collapsed summary")));
    assert!(texts.iter().any(|text| text == "kept-1"));
    assert!(texts.iter().any(|text| text == "kept-2"));
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .start_index,
        3
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_relinks_id_based_runtime_state_when_message_ids_change() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-relink-id-change".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("old-1").with_text("prefix"),
        Message::assistant()
            .with_id("old-2")
            .with_text("collapsed-1"),
        Message::user().with_id("old-3").with_text("collapsed-2"),
        Message::assistant().with_id("old-4").with_text("kept-1"),
        Message::user().with_id("old-5").with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-relink-id-change".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: Some("old-2".to_string()),
        end_message_id: Some("old-3".to_string()),
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 1,
        head_index: 1,
        tail_index: 3,
        direction: CompactDirection::UpTo,
        anchor_message_id: Some("old-2".to_string()),
        head_message_id: Some("old-2".to_string()),
        tail_message_id: Some("old-4".to_string()),
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 3,
        end_index: 4,
        tail_anchor_index: 4,
        start_message_id: Some("old-4".to_string()),
        end_message_id: Some("old-5".to_string()),
        tail_anchor_message_id: Some("old-5".to_string()),
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("new-0").with_text("prefix"),
        Message::assistant()
            .with_id("new-1")
            .with_text("collapsed-1"),
        Message::user().with_id("new-2").with_text("collapsed-2"),
        Message::assistant().with_id("new-3").with_text("kept-1"),
        Message::user().with_id("new-4").with_text("kept-2"),
        Message::assistant().with_id("new-5").with_text("new-tail"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state =
        ContextRuntimeState::from_extension_data(&loaded.extension_data).expect("runtime state");
    let projected =
        project_for_provider(&loaded.conversation.expect("conversation"), &loaded_state);
    let texts = projected
        .messages()
        .iter()
        .map(|message| message.as_concat_text())
        .collect::<Vec<_>>();

    assert!(texts.iter().any(|text| text.contains("collapsed summary")));
    assert!(texts.iter().any(|text| text == "kept-1"));
    assert!(texts.iter().any(|text| text == "kept-2"));
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .start_index,
        3
    );
    assert_eq!(
        loaded_state.committed_collapses()[0]
            .start_message_id
            .as_deref(),
        Some("new-1")
    );
    assert_eq!(
        loaded_state.committed_collapses()[0]
            .end_message_id
            .as_deref(),
        Some("new-2")
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_relinks_runtime_state_when_old_indices_are_stale_but_message_ids_are_valid(
) {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-relink-stale-indices".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("old-1").with_text("prefix"),
        Message::assistant()
            .with_id("old-2")
            .with_text("collapsed-1"),
        Message::user().with_id("old-3").with_text("collapsed-2"),
        Message::assistant().with_id("old-4").with_text("kept-1"),
        Message::user().with_id("old-5").with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-stale-indices".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 0,
        end_index: 2,
        direction: CompactDirection::UpTo,
        start_message_id: Some("old-2".to_string()),
        end_message_id: Some("old-3".to_string()),
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 0,
        head_index: 0,
        tail_index: 2,
        direction: CompactDirection::UpTo,
        anchor_message_id: Some("old-2".to_string()),
        head_message_id: Some("old-2".to_string()),
        tail_message_id: Some("old-4".to_string()),
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 2,
        end_index: 4,
        tail_anchor_index: 4,
        start_message_id: Some("old-4".to_string()),
        end_message_id: Some("old-5".to_string()),
        tail_anchor_message_id: Some("old-5".to_string()),
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("new-0").with_text("prefix"),
        Message::assistant()
            .with_id("new-1")
            .with_text("collapsed-1"),
        Message::user().with_id("new-2").with_text("collapsed-2"),
        Message::assistant().with_id("new-3").with_text("kept-1"),
        Message::user().with_id("new-4").with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state =
        ContextRuntimeState::from_extension_data(&loaded.extension_data).expect("runtime state");

    assert_eq!(loaded_state.committed_collapses()[0].start_index, 1);
    assert_eq!(loaded_state.committed_collapses()[0].end_index, 3);
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .start_index,
        3
    );
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .end_index,
        4
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_invalidates_committed_collapse_when_prefix_changes() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-collapse-prefix-mismatch".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("old-1").with_text("prefix"),
        Message::assistant()
            .with_id("old-2")
            .with_text("collapsed-1"),
        Message::user().with_id("old-3").with_text("collapsed-2"),
        Message::assistant().with_id("old-4").with_text("kept-1"),
        Message::user().with_id("old-5").with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-prefix-mismatch".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: Some("old-2".to_string()),
        end_message_id: Some("old-3".to_string()),
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 1,
        head_index: 1,
        tail_index: 3,
        direction: CompactDirection::UpTo,
        anchor_message_id: Some("old-2".to_string()),
        head_message_id: Some("old-2".to_string()),
        tail_message_id: Some("old-4".to_string()),
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 3,
        end_index: 4,
        tail_anchor_index: 4,
        start_message_id: Some("old-4".to_string()),
        end_message_id: Some("old-5".to_string()),
        tail_anchor_message_id: Some("old-5".to_string()),
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("new-0").with_text("new-prefix"),
        Message::assistant()
            .with_id("new-1")
            .with_text("collapsed-1"),
        Message::user().with_id("new-2").with_text("collapsed-2"),
        Message::assistant().with_id("new-3").with_text("kept-1"),
        Message::user().with_id("new-4").with_text("kept-2"),
        Message::assistant().with_id("new-5").with_text("new-tail"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data);
    assert!(
        loaded_state
            .as_ref()
            .is_none_or(|state| state.committed_collapses().is_empty()),
        "committed collapse should not survive when unsummarized prefix changes"
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_relinks_from_direction_runtime_state_when_suffix_unchanged() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-from-relink".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("old-1").with_text("head-1"),
        Message::assistant().with_id("old-2").with_text("head-2"),
        Message::user().with_id("old-3").with_text("head-3"),
        Message::assistant().with_id("old-4").with_text("tail-1"),
        Message::user().with_id("old-5").with_text("tail-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-from-relink".to_string()),
        summary: "suffix summary".to_string(),
        start_index: 3,
        end_index: 5,
        direction: CompactDirection::From,
        start_message_id: Some("old-4".to_string()),
        end_message_id: Some("old-5".to_string()),
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 3,
        head_index: 2,
        tail_index: 4,
        direction: CompactDirection::From,
        anchor_message_id: Some("old-4".to_string()),
        head_message_id: Some("old-3".to_string()),
        tail_message_id: Some("old-5".to_string()),
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 0,
        end_index: 2,
        tail_anchor_index: 2,
        start_message_id: Some("old-1".to_string()),
        end_message_id: Some("old-3".to_string()),
        tail_anchor_message_id: Some("old-3".to_string()),
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("new-1").with_text("head-1"),
        Message::assistant().with_id("new-2").with_text("head-2"),
        Message::user().with_id("new-3").with_text("head-3"),
        Message::assistant().with_id("new-4").with_text("tail-1"),
        Message::user().with_id("new-5").with_text("tail-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state =
        ContextRuntimeState::from_extension_data(&loaded.extension_data).expect("runtime state");

    assert_eq!(
        loaded_state.committed_collapses()[0].direction,
        CompactDirection::From
    );
    assert_eq!(loaded_state.committed_collapses()[0].start_index, 3);
    assert_eq!(loaded_state.committed_collapses()[0].end_index, 5);
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .end_index,
        2
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_invalidates_from_direction_runtime_state_when_suffix_changes() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-from-invalidate".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("old-1").with_text("head-1"),
        Message::assistant().with_id("old-2").with_text("head-2"),
        Message::user().with_id("old-3").with_text("head-3"),
        Message::assistant().with_id("old-4").with_text("tail-1"),
        Message::user().with_id("old-5").with_text("tail-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-from-invalidate".to_string()),
        summary: "suffix summary".to_string(),
        start_index: 3,
        end_index: 5,
        direction: CompactDirection::From,
        start_message_id: Some("old-4".to_string()),
        end_message_id: Some("old-5".to_string()),
        created_at: 1,
    });
    state.set_compact_boundary(Some(CompactBoundaryMetadata {
        anchor_index: 3,
        head_index: 2,
        tail_index: 4,
        direction: CompactDirection::From,
        anchor_message_id: Some("old-4".to_string()),
        head_message_id: Some("old-3".to_string()),
        tail_message_id: Some("old-5".to_string()),
        created_at: 1,
    }));
    state.set_preserved_segment(Some(PreservedSegmentMetadata {
        start_index: 0,
        end_index: 2,
        tail_anchor_index: 2,
        start_message_id: Some("old-1".to_string()),
        end_message_id: Some("old-3".to_string()),
        tail_anchor_message_id: Some("old-3".to_string()),
        reason: "committed_collapse".to_string(),
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_id("new-1").with_text("head-1"),
        Message::assistant().with_id("new-2").with_text("head-2"),
        Message::user().with_id("new-3").with_text("head-3"),
        Message::assistant().with_id("new-4").with_text("tail-1"),
        Message::user().with_id("new-5").with_text("tail-2"),
        Message::assistant().with_id("new-6").with_text("tail-3"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data);
    assert!(
        loaded_state
            .as_ref()
            .is_none_or(|state| state.committed_collapses().is_empty()),
        "from-direction collapse should not survive when collapsed suffix changes"
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_clears_unrelinkable_idless_runtime_state() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-clear-unrelinkable".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("alpha"),
        Message::assistant().with_text("beta"),
        Message::user().with_text("gamma"),
        Message::assistant().with_text("delta"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-unrelinkable".to_string()),
        summary: "old collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("one"),
        Message::assistant().with_text("two"),
        Message::user().with_text("three"),
        Message::assistant().with_text("four"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, false)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data);
    assert!(
        loaded_state.as_ref().is_none_or(|state| {
            state.committed_collapses().is_empty()
                && state.session_memory().is_none()
                && state.staged_snapshot().is_none()
        }),
        "unrelinkable idless structural runtime state should not survive replacement"
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_relinks_idless_session_memory_by_message_signature() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-memory-relink".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.set_session_memory(Some(agime::context_runtime::SessionMemoryState {
        summary: "memory summary".to_string(),
        summarized_through_message_id: None,
        preserved_start_index: 3,
        preserved_end_index: 4,
        preserved_start_message_id: None,
        preserved_end_message_id: None,
        preserved_message_count: 2,
        preserved_token_estimate: 10,
        tail_anchor_index: 4,
        tail_anchor_message_id: None,
        updated_at: 1,
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
        Message::assistant().with_text("new-tail"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state =
        ContextRuntimeState::from_extension_data(&loaded.extension_data).expect("runtime state");
    let memory = loaded_state.session_memory().expect("session memory");
    assert_eq!(memory.preserved_start_index, 3);
    assert_eq!(memory.preserved_end_index, 5);
    assert_eq!(memory.tail_anchor_index, 5);
    assert_eq!(memory.preserved_message_count, 3);
    let projected =
        project_for_provider(&loaded.conversation.expect("conversation"), &loaded_state);
    let texts = projected
        .messages()
        .iter()
        .map(|message| message.as_concat_text())
        .collect::<Vec<_>>();
    assert!(texts.iter().any(|text| text.contains("memory summary")));
    assert!(texts.iter().any(|text| text == "kept-1"));
    assert!(texts.iter().any(|text| text == "kept-2"));
    assert!(texts.iter().any(|text| text == "new-tail"));

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_invalidates_session_memory_when_summarized_prefix_changes() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-memory-prefix-mismatch".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.set_session_memory(Some(agime::context_runtime::SessionMemoryState {
        summary: "memory summary".to_string(),
        summarized_through_message_id: None,
        preserved_start_index: 3,
        preserved_end_index: 4,
        preserved_start_message_id: None,
        preserved_end_message_id: None,
        preserved_message_count: 2,
        preserved_token_estimate: 10,
        tail_anchor_index: 4,
        tail_anchor_message_id: None,
        updated_at: 1,
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("new-prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
        Message::assistant().with_text("new-tail"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data);
    assert!(
        loaded_state
            .as_ref()
            .is_none_or(|state| state.session_memory().is_none()),
        "session memory should not survive when summarized prefix changes"
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_invalidates_staged_snapshot_runtime_state() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-stage-invalidate".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("one"),
        Message::assistant().with_text("two"),
        Message::user().with_text("three"),
        Message::assistant().with_text("four"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.set_staged_snapshot(Some(agime::context_runtime::CollapseSnapshot {
        snapshot_id: Some("stage-1".to_string()),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        summary: "staged summary".to_string(),
        risk: 0.7,
        staged_at: 1,
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    let replaced_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("new-one"),
        Message::assistant().with_text("new-two"),
        Message::user().with_text("new-three"),
    ]);
    SessionManager::replace_conversation(&session.id, &replaced_conversation)
        .await
        .expect("replace conversation");

    let loaded = SessionManager::get_session(&session.id, false)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data);
    assert!(
        loaded_state
            .as_ref()
            .is_none_or(|state| state.staged_snapshot().is_none()),
        "staged snapshot should not survive conversation replacement"
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn replace_conversation_to_empty_clears_structural_runtime_state() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-empty-target".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let old_conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("one"),
        Message::assistant().with_text("two"),
        Message::user().with_text("three"),
        Message::assistant().with_text("four"),
    ]);
    SessionManager::replace_conversation(&session.id, &old_conversation)
        .await
        .expect("seed old conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-empty-target".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.set_session_memory(Some(agime::context_runtime::SessionMemoryState {
        summary: "memory summary".to_string(),
        summarized_through_message_id: None,
        preserved_start_index: 2,
        preserved_end_index: 3,
        preserved_start_message_id: None,
        preserved_end_message_id: None,
        preserved_message_count: 2,
        preserved_token_estimate: 10,
        tail_anchor_index: 3,
        tail_anchor_message_id: None,
        updated_at: 1,
    }));
    state.set_staged_snapshot(Some(agime::context_runtime::CollapseSnapshot {
        snapshot_id: Some("stage-empty-target".to_string()),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        summary: "staged summary".to_string(),
        risk: 0.5,
        staged_at: 1,
    }));
    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist state");

    SessionManager::replace_conversation(&session.id, &Conversation::default())
        .await
        .expect("replace with empty conversation");

    let loaded = SessionManager::get_session(&session.id, false)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data);
    assert!(
        loaded_state.as_ref().is_none_or(|state| {
            state.committed_collapses().is_empty()
                && state.session_memory().is_none()
                && state.staged_snapshot().is_none()
        }),
        "structural runtime state should clear when conversation is replaced with empty"
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn set_extension_state_refreshes_context_runtime_projection_cache() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-set-extension-refresh".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    let mut state = ContextRuntimeState::default();
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-refresh".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state.last_projection_stats = Some(agime::context_runtime::ProjectionStats {
        base_agent_messages: 999,
        projected_agent_messages: 999,
        snip_removed_count: 999,
        microcompacted_count: 999,
        raw_token_estimate: 9999,
        projected_token_estimate: 9999,
        freed_token_estimate: 0,
        updated_at: 1,
    });

    SessionManager::set_extension_state(&session.id, &state)
        .await
        .expect("persist runtime state");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data)
        .expect("load context runtime state");

    assert_eq!(
        loaded_state
            .compact_boundary()
            .expect("boundary")
            .tail_index,
        3
    );
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .start_index,
        3
    );
    assert_ne!(
        loaded_state
            .last_projection_stats
            .as_ref()
            .expect("projection stats")
            .raw_token_estimate,
        9999
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}

#[tokio::test]
#[serial]
async fn update_session_extension_data_refreshes_context_runtime_projection_cache() {
    let session = SessionManager::create_session(
        std::env::temp_dir(),
        "context-runtime-update-extension-refresh".to_string(),
        SessionType::Hidden,
    )
    .await
    .expect("create session");

    let conversation = Conversation::new_unvalidated(vec![
        Message::user().with_text("prefix"),
        Message::assistant().with_text("collapsed-1"),
        Message::user().with_text("collapsed-2"),
        Message::assistant().with_text("kept-1"),
        Message::user().with_text("kept-2"),
    ]);
    SessionManager::replace_conversation(&session.id, &conversation)
        .await
        .expect("replace conversation");

    let mut extension_data = ExtensionData::new();
    let mut state = ContextRuntimeState {
        last_projection_stats: Some(agime::context_runtime::ProjectionStats {
            base_agent_messages: 999,
            projected_agent_messages: 999,
            snip_removed_count: 999,
            microcompacted_count: 999,
            raw_token_estimate: 9999,
            projected_token_estimate: 9999,
            freed_token_estimate: 0,
            updated_at: 1,
        }),
        ..ContextRuntimeState::default()
    };
    state.append_collapse_commit(CollapseCommit {
        commit_id: Some("commit-refresh".to_string()),
        summary: "collapsed summary".to_string(),
        start_index: 1,
        end_index: 3,
        direction: CompactDirection::UpTo,
        start_message_id: None,
        end_message_id: None,
        created_at: 1,
    });
    state
        .to_extension_data(&mut extension_data)
        .expect("serialize runtime state");

    SessionManager::update_session(&session.id)
        .extension_data(extension_data)
        .apply()
        .await
        .expect("persist extension data");

    let loaded = SessionManager::get_session(&session.id, true)
        .await
        .expect("load session");
    let loaded_state = ContextRuntimeState::from_extension_data(&loaded.extension_data)
        .expect("load context runtime state");

    assert_eq!(
        loaded_state
            .compact_boundary()
            .expect("boundary")
            .tail_index,
        3
    );
    assert_eq!(
        loaded_state
            .preserved_segment()
            .expect("preserved segment")
            .start_index,
        3
    );
    assert_ne!(
        loaded_state
            .last_projection_stats
            .as_ref()
            .expect("projection stats")
            .raw_token_estimate,
        9999
    );

    SessionManager::delete_session(&session.id)
        .await
        .expect("delete session");
}
