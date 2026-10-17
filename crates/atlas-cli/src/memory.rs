use super::*;

pub(crate) async fn cmd_memory(action: MemoryAction) -> anyhow::Result<()> {
    let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let engine = MemoryEngine::from_workspace(&base_dir);
    match action {
        MemoryAction::Status { json } => {
            let status = engine.status().await;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("{} — Dual-Memory Stack Status", atlas_core::COMPANY_NAME);
                println!("=====================================");
                println!("1. Short-Term Memory: Shared Context MCP");
                println!("   Endpoint: {}", status.shared_context.endpoint);
                println!("   Mode:     {:?}", status.shared_context.mode);
                println!(
                    "   Status:   {}",
                    if status.shared_context.connected {
                        "CONNECTED (OK)"
                    } else {
                        "OFFLINE / LOCAL FALLBACK"
                    }
                );
                if let Some(lat) = status.shared_context.latency_ms {
                    println!("   Latency:  {} ms", lat);
                }
                println!(
                    "   Active Tasks:          {}",
                    status.shared_context.active_tasks
                );
                println!(
                    "   Unpromoted Completed:  {}",
                    status.shared_context.completed_unpromoted_tasks
                );
                println!(
                    "   Active Swarm Agents:   {}",
                    status.shared_context.active_agents
                );
                println!();
                println!("2. Long-Term Memory: Graphiti Knowledge Graph");
                println!("   Endpoint: {}", status.graphiti.endpoint);
                println!("   Group ID: {}", status.graphiti.group_id);
                println!(
                    "   Status:   {}",
                    if status.graphiti.connected {
                        "CONNECTED (OK)"
                    } else {
                        "OFFLINE / LOCAL GRAPH FALLBACK"
                    }
                );
                if let Some(lat) = status.graphiti.latency_ms {
                    println!("   Latency:  {} ms", lat);
                }
                println!(
                    "   Total Episodes:        {}",
                    status.graphiti.total_episodes
                );
                println!(
                    "   Indexed Entities:      {}",
                    status.graphiti.total_entities
                );
                println!("   Knowledge Edges:       {}", status.graphiti.total_edges);
                println!();
                println!("Promotion Policy:");
                println!(
                    "   Auto-promote on complete: {}",
                    status.auto_promote_enabled
                );
                println!(
                    "   Ephemeral retention:      {} days",
                    status.prune_retention_days
                );
            }
        }
        MemoryAction::Recall { query, limit, json } => {
            let results = engine.graphiti().search(&query, limit).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                println!("Graphiti Knowledge Graph Recall for: '{}'", query);
                println!("============================================");
                if results.is_empty() {
                    println!("No matching episodes or entities found.");
                } else {
                    for (i, r) in results.iter().enumerate() {
                        println!("[{}] {} (Relevance: {:.2})", i + 1, r.title, r.score);
                        println!("    Created:  {}", r.created_at.to_rfc3339());
                        println!("    Summary:  {}", r.summary);
                        if !r.matched_entities.is_empty() {
                            println!("    Entities: {}", r.matched_entities.join(", "));
                        }
                        println!();
                    }
                }
            }
        }
        MemoryAction::Tasks { status, json } => {
            let filter = status.and_then(|s| match s.to_lowercase().as_str() {
                "pending" => Some(MemoryTaskStatus::Pending),
                "in_progress" => Some(MemoryTaskStatus::InProgress),
                "review" => Some(MemoryTaskStatus::Review),
                "completed" => Some(MemoryTaskStatus::Completed),
                "failed" => Some(MemoryTaskStatus::Failed),
                _ => None,
            });
            let tasks = engine.shared_context().list_tasks(filter).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&tasks)?);
            } else {
                println!("Shared Context MCP — Working Memory Task Board");
                println!("=============================================");
                if tasks.is_empty() {
                    println!("No active tasks in working memory.");
                } else {
                    println!(
                        "{:<14} {:<24} {:<12} {:<10} {:<8}",
                        "TASK ID", "TITLE", "ASSIGNED", "STATUS", "PROMOTED"
                    );
                    for t in tasks {
                        println!(
                            "{:<14} {:<24} {:<12} {:<10} {:<8}",
                            t.task_id,
                            if t.title.len() > 22 {
                                format!("{}...", &t.title[..22])
                            } else {
                                t.title
                            },
                            t.assigned_to.unwrap_or_else(|| "unassigned".to_string()),
                            format!("{:?}", t.status),
                            if t.promoted_to_graphiti { "YES" } else { "NO" }
                        );
                    }
                }
            }
        }
        MemoryAction::CreateTask {
            title,
            description,
            agent,
        } => {
            let task = engine
                .shared_context()
                .create_task(&title, &description, agent.as_deref())
                .await?;
            println!("Created task in Shared Context: {}", task.task_id);
            println!("  Title:       {}", task.title);
            println!(
                "  Assigned to: {}",
                task.assigned_to.unwrap_or_else(|| "unassigned".to_string())
            );
        }
        MemoryAction::AddNote { task, note } => {
            let ok = engine
                .shared_context()
                .add_context_note(&task, &note)
                .await?;
            if ok {
                println!("Added context note to task {}", task);
            } else {
                eprintln!("Task not found: {}", task);
            }
        }
        MemoryAction::UpdateTask { task, status } => {
            let parsed = match status.to_lowercase().as_str() {
                "pending" => MemoryTaskStatus::Pending,
                "in_progress" => MemoryTaskStatus::InProgress,
                "review" => MemoryTaskStatus::Review,
                "completed" => MemoryTaskStatus::Completed,
                "failed" => MemoryTaskStatus::Failed,
                _ => anyhow::bail!(
                    "Invalid status: {}. Use pending, in_progress, review, completed, or failed.",
                    status
                ),
            };
            let updated = engine
                .shared_context()
                .update_task_status(&task, parsed)
                .await?;
            if let Some(t) = updated {
                println!("Updated task {} status to {:?}", t.task_id, t.status);
            } else {
                eprintln!("Task not found: {}", task);
            }
        }
        MemoryAction::Promote { task, notes, json } => {
            let report = engine
                .promoter()
                .promote_task(&task, notes.as_deref())
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Promoted Task to Graphiti Knowledge Graph");
                println!("=========================================");
                println!("Task ID:         {}", report.task_id);
                println!("Title:           {}", report.task_title);
                println!("Graph Episode:   {}", report.episode_id);
                println!("Artifacts Bound: {}", report.artifact_count);
                println!("Notes Distilled: {}", report.note_count);
                println!("Promoted At:     {}", report.promoted_at.to_rfc3339());
                println!();
                println!("Distilled Knowledge Payload:");
                println!("{}", report.distilled_summary);
            }
        }
        MemoryAction::AutoPromote { json } => {
            let reports = engine.promoter().auto_promote_completed().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&reports)?);
            } else {
                println!(
                    "Auto-promoted {} completed task(s) into Graphiti:",
                    reports.len()
                );
                for r in reports {
                    println!(
                        "  - [{}] {} -> Episode {}",
                        r.task_id, r.task_title, r.episode_id
                    );
                }
            }
        }
        MemoryAction::Preload { topic, agent } => {
            let (task, matches) = engine
                .promoter()
                .bootstrap_topic_context(&topic, agent.as_deref())
                .await?;
            println!("Bootstrapped Shared Context with Graphiti Memory");
            println!("===============================================");
            println!("Created Working Task: {}", task.task_id);
            println!(
                "Assigned Agent:       {}",
                task.assigned_to.unwrap_or_else(|| "none".to_string())
            );
            println!("Recalled Episodes:    {}", matches.len());
            for m in matches {
                println!("  • [Score: {:.2}] {}: {}", m.score, m.title, m.summary);
            }
        }
        MemoryAction::Prune { days } => {
            let count = engine.shared_context().prune_ephemeral(days).await?;
            println!(
                "Pruned {} stale ephemeral task records older than {} days.",
                count, days
            );
        }
        MemoryAction::Daemon { poll_seconds, once } => {
            println!("Starting Dual-Memory Bridge Daemon (Shared Context <-> Graphiti)...");
            println!("Polling every {}s. Press Ctrl+C to stop.", poll_seconds);
            loop {
                let reports = engine.promoter().auto_promote_completed().await?;
                if !reports.is_empty() {
                    for r in reports {
                        println!(
                            "[Auto-Promote] Committed task {} ({}) -> Episode {}",
                            r.task_id, r.task_title, r.episode_id
                        );
                    }
                }
                if once {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(poll_seconds)).await;
            }
        }
    }
    Ok(())
}
