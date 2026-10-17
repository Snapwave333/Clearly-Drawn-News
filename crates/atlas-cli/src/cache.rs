use super::*;

pub(crate) const CACHE_AREAS: [(&str, &str); 4] = [
    ("narration", "audio/segments"),
    ("renders", "renders"),
    ("broll", "media"),
    ("vectors", "vectors"),
];

pub(crate) struct CacheEntry {
    path: PathBuf,
    bytes: u64,
    newest: std::time::SystemTime,
}

pub(crate) fn scan_cache_area(root: &Path) -> Vec<CacheEntry> {
    let Ok(children) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    children
        .filter_map(Result::ok)
        .map(|child| {
            let path = child.path();
            let (bytes, newest) = tree_usage(&path);
            CacheEntry { path, bytes, newest }
        })
        .collect()
}

pub(crate) fn tree_usage(path: &Path) -> (u64, std::time::SystemTime) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return (0, std::time::UNIX_EPOCH);
    };
    let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
    if !metadata.is_dir() {
        return (metadata.len(), modified);
    }
    let mut bytes = 0;
    let mut newest = None;
    if let Ok(children) = std::fs::read_dir(path) {
        for child in children.filter_map(Result::ok) {
            let (child_bytes, child_newest) = tree_usage(&child.path());
            bytes += child_bytes;
            newest = newest.max(Some(child_newest));
        }
    }
    (bytes, newest.unwrap_or(modified))
}

pub(crate) fn prune_cache(
    data_dir: &Path,
    cutoff: std::time::SystemTime,
    dry_run: bool,
) -> anyhow::Result<(usize, u64)> {
    let mut removed = 0;
    let mut freed = 0;
    for (_, relative) in CACHE_AREAS {
        for entry in scan_cache_area(&data_dir.join(relative)) {
            if entry.newest >= cutoff {
                continue;
            }
            if !dry_run {
                if entry.path.is_dir() {
                    std::fs::remove_dir_all(&entry.path)?;
                } else {
                    std::fs::remove_file(&entry.path)?;
                }
            }
            removed += 1;
            freed += entry.bytes;
        }
    }
    Ok((removed, freed))
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub(crate) fn cmd_cache(store: &Store, action: CacheAction) -> anyhow::Result<()> {
    let data_dir = store.data_dir();
    match action {
        CacheAction::Stats => {
            println!("{} — Cache Usage ({})", atlas_core::COMPANY_NAME, data_dir.display());
            let mut total = 0;
            for (name, relative) in CACHE_AREAS {
                let entries = scan_cache_area(&data_dir.join(relative));
                let bytes = entries.iter().map(|entry| entry.bytes).sum::<u64>();
                total += bytes;
                println!("  {:10} {:>6} entries {:>12}", name, entries.len(), format_bytes(bytes));
            }
            println!("  {:10} {:>21}", "total", format_bytes(total));
        }
        CacheAction::Prune {
            older_than_days,
            dry_run,
        } => {
            let cutoff = std::time::SystemTime::now()
                - std::time::Duration::from_secs(older_than_days.saturating_mul(86_400));
            let (removed, freed) = prune_cache(data_dir, cutoff, dry_run)?;
            let verb = if dry_run { "Would remove" } else { "Removed" };
            println!(
                "{verb} {removed} cache entries older than {older_than_days} days, freeing {}",
                format_bytes(freed)
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_removes_only_stale_cache_entries() {
        let dir = std::env::temp_dir().join(format!("atlas_cache_{}", uuid::Uuid::new_v4().simple()));
        let stale = dir.join("renders").join("old_long").join("shot_001");
        let fresh = dir.join("renders").join("new_long");
        let export = dir.join("exports").join("old").join("final.mp4");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::create_dir_all(export.parent().unwrap()).unwrap();
        std::fs::write(stale.join("0001.png"), vec![0_u8; 2048]).unwrap();
        std::fs::write(fresh.join("0001.png"), b"new").unwrap();
        std::fs::write(&export, b"master").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(90 * 86_400);
        std::fs::File::options()
            .write(true)
            .open(stale.join("0001.png"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86_400);

        assert_eq!(prune_cache(&dir, cutoff, true).unwrap(), (1, 2048));
        assert!(stale.is_dir());
        assert_eq!(prune_cache(&dir, cutoff, false).unwrap(), (1, 2048));
        assert!(!stale.exists());
        assert!(fresh.join("0001.png").is_file());
        assert!(export.is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
