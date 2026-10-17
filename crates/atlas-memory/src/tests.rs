use crate::MemoryEngine;
use crate::config::{MemoryConfig, SharedContextMode};
use crate::shared_context::TaskStatus;

#[tokio::test]
async fn test_shared_context_lifecycle() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        shared_context_mode: SharedContextMode::LocalBridge,
        local_cache_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    };
    let engine = MemoryEngine::new(config);
    let sc = engine.shared_context();

    let task = sc
        .create_task(
            "Test Whiteboard Shot",
            "Render scene 01 with Blender",
            Some("OpenCode"),
        )
        .await
        .unwrap();
    assert_eq!(task.status, TaskStatus::Pending);
    assert_eq!(task.assigned_to.as_deref(), Some("OpenCode"));

    sc.add_context_note(&task.task_id, "Configured Grease Pencil stroke reveals")
        .await
        .unwrap();

    sc.add_artifact(&task.task_id, "artifacts/shot01.blend")
        .await
        .unwrap();

    let updated = sc
        .update_task_status(&task.task_id, TaskStatus::Completed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, TaskStatus::Completed);
    assert!(!updated.promoted_to_graphiti);
}

#[tokio::test]
async fn test_graphiti_add_and_search() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        local_cache_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    };
    let engine = MemoryEngine::new(config);
    let gr = engine.graphiti();

    let ep = gr
        .add_episode(
            "SQLite WAL Concurrency",
            "Enabled WAL mode with synchronous=NORMAL to prevent lock contention with multiple workers.",
            "Hermes agent benchmark",
        )
        .await
        .unwrap();

    assert_eq!(ep.name, "SQLite WAL Concurrency");

    let results = gr.search("SQLite WAL", 5).await.unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].title, "SQLite WAL Concurrency");
}

#[tokio::test]
async fn test_promotion_engine_flow() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        local_cache_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    };
    let engine = MemoryEngine::new(config);
    let sc = engine.shared_context();
    let gr = engine.graphiti();
    let promoter = engine.promoter();

    let task = sc
        .create_task(
            "Kokoro Audio Normalization",
            "Adjusted LUFS to -14 for YouTube",
            Some("Hermes"),
        )
        .await
        .unwrap();

    sc.add_context_note(
        &task.task_id,
        "Applied loudnorm filter with 2-pass analysis",
    )
    .await
    .unwrap();
    sc.add_artifact(&task.task_id, "output/narration_master.wav")
        .await
        .unwrap();
    sc.update_task_status(&task.task_id, TaskStatus::Completed)
        .await
        .unwrap();

    let report = promoter
        .promote_task(
            &task.task_id,
            Some("Never skip pass 1 or dynamic range will clip"),
        )
        .await
        .unwrap();

    assert_eq!(report.task_id, task.task_id);
    assert_eq!(report.artifact_count, 1);
    assert_eq!(report.note_count, 1);

    let promoted_task = sc.get_task(&task.task_id).await.unwrap().unwrap();
    assert!(promoted_task.promoted_to_graphiti);

    let recalled = gr.search("Kokoro LUFS", 5).await.unwrap();
    assert!(!recalled.is_empty());
    assert!(recalled[0].summary.contains("loudnorm"));

    let (bootstrap_task, matches) = promoter
        .bootstrap_topic_context("Kokoro Audio", Some("OpenCode"))
        .await
        .unwrap();

    println!(
        "MATCHES FOUND: {:?}",
        matches.iter().map(|m| &m.title).collect::<Vec<_>>()
    );
    assert!(!matches.is_empty());
    assert_eq!(bootstrap_task.assigned_to.as_deref(), Some("OpenCode"));
    assert!(!bootstrap_task.context_notes.is_empty());
}
