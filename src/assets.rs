use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};
use zip::ZipArchive;

const MAX_ENTRY: u64 = 128 * 1024 * 1024;

fn save_fingerprint_start() -> Option<Instant> {
    #[cfg(target_os = "horizon")]
    {
        Some(Instant::now())
    }
    #[cfg(not(target_os = "horizon"))]
    {
        None
    }
}

fn save_fingerprint_marker(stage: &str, started: Option<&Instant>) {
    #[cfg(target_os = "horizon")]
    if let Some(started) = started {
        crate::horizon_diag(&format!(
            "rust:save-fingerprint:{stage}:elapsed-ms={}",
            started.elapsed().as_millis()
        ));
    }
    #[cfg(not(target_os = "horizon"))]
    let _ = (stage, started);
}

fn save_fingerprint_progress(stage: &str, entries: usize, started: Option<&Instant>) {
    #[cfg(target_os = "horizon")]
    if let Some(started) = started {
        crate::horizon_diag(&format!(
            "rust:save-fingerprint:{stage}:entries={entries}:elapsed-ms={}",
            started.elapsed().as_millis()
        ));
    }
    #[cfg(not(target_os = "horizon"))]
    let _ = (stage, entries, started);
}

fn save_fingerprint_variant_marker(variant: &str, phase: &str, started: Option<&Instant>) {
    #[cfg(target_os = "horizon")]
    if let Some(started) = started {
        crate::horizon_diag(&format!(
            "rust:save-fingerprint:{variant}:{phase}:elapsed-ms={}",
            started.elapsed().as_millis()
        ));
    }
    #[cfg(not(target_os = "horizon"))]
    let _ = (variant, phase, started);
}

fn save_fingerprint_variant(index: usize) -> Option<&'static str> {
    match index {
        0 => Some("variant-0"),
        1 => Some("variant-1"),
        2 => Some("variant-2"),
        _ => None,
    }
}

/// Read-only PK3 mount. Later alphabetically sorted packs override earlier packs.
/// Archive files must remain unchanged while this mount is in use; core
/// save identity reuses the metadata captured by this mount.
pub struct Assets {
    packs: Vec<ZipArchive<File>>,
    paths: Vec<PathBuf>,
    entries: BTreeMap<String, (usize, usize)>,
    // Same alphabetical override rules as `entries`, with language packs
    // omitted. Built during the initial archive walk so save identity does
    // not reopen and re-enumerate every core archive.
    core_entries: BTreeMap<String, (usize, usize)>,
    pub base: PathBuf,
}

impl Assets {
    /// Stable identity of the mounted, override-resolved corpus, independent of its folder.
    /// Central-directory CRC/size metadata avoids decoding every asset at startup.
    pub fn fingerprint(&mut self) -> Result<String> {
        self.fingerprint_with_progress(None)
    }
    fn fingerprint_with_progress(
        &mut self,
        diagnostic: Option<(&str, &Instant)>,
    ) -> Result<String> {
        Self::fingerprint_entries(&mut self.packs, &self.entries, diagnostic)
    }
    fn fingerprint_entries(
        packs: &mut [ZipArchive<File>],
        entries: &BTreeMap<String, (usize, usize)>,
        diagnostic: Option<(&str, &Instant)>,
    ) -> Result<String> {
        use sha2::{Digest, Sha256};
        // Bound progress output to three numeric checkpoints per fingerprint.
        const PROGRESS_ENTRIES: [usize; 3] = [4096, 16384, 65536];
        let mut hash = Sha256::new();
        let mut next_progress = 0;
        for (index_in_map, (name, &(pack, index))) in entries.iter().enumerate() {
            let entry = packs[pack].by_index(index)?;
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update(entry.size().to_le_bytes());
            hash.update(entry.crc32().to_le_bytes());
            let processed = index_in_map + 1;
            if let Some((stage, started)) = diagnostic {
                if next_progress < PROGRESS_ENTRIES.len()
                    && processed >= PROGRESS_ENTRIES[next_progress]
                {
                    save_fingerprint_progress(
                        stage,
                        PROGRESS_ENTRIES[next_progress],
                        Some(started),
                    );
                    next_progress += 1;
                }
            }
        }
        Ok(format!("{:x}", hash.finalize()))
    }
    pub fn open(base: &Path) -> Result<Self> {
        let paths = fs::read_dir(base)
            .with_context(|| {
                format!(
                    "Cannot read game data at {}. Pass --data <base folder>.",
                    base.display()
                )
            })?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("pk3")))
            .collect::<Vec<_>>();
        Self::open_paths(base, paths)
    }

    fn open_paths(base: &Path, mut paths: Vec<PathBuf>) -> Result<Self> {
        Self::open_paths_with_core_index(base, paths, true)
    }

    fn open_variant_paths(base: &Path, paths: Vec<PathBuf>) -> Result<Self> {
        Self::open_paths_with_core_index(base, paths, false)
    }

    fn open_paths_with_core_index(
        base: &Path,
        mut paths: Vec<PathBuf>,
        cache_core_index: bool,
    ) -> Result<Self> {
        paths.sort_by_key(|p| p.file_name().unwrap_or_default().to_ascii_lowercase());
        if paths.is_empty() {
            bail!("No PK3 archives found in {}", base.display());
        }
        let mut entries = BTreeMap::new();
        let mut core_entries = BTreeMap::new();
        let mut packs = Vec::new();
        for path in &paths {
            let mut pack = ZipArchive::new(File::open(&path)?)
                .with_context(|| format!("Invalid PK3: {}", path.display()))?;
            let p = packs.len();
            let is_core = !is_language_pack(path);
            for i in 0..pack.len() {
                let entry = pack.by_index(i)?;
                if !entry.is_dir() {
                    let name = normalize(entry.name());
                    entries.insert(name.clone(), (p, i));
                    if is_core && cache_core_index {
                        core_entries.insert(name, (p, i));
                    }
                }
            }
            packs.push(pack);
        }
        Ok(Self {
            packs,
            paths,
            entries,
            core_entries,
            base: base.to_path_buf(),
        })
    }

    /// Saves identify core data independently of all known language packs.
    /// Reconstruct older identities with and without English, using the same
    /// core packs and available translations. Runtime mounting is unchanged.
    pub(crate) fn save_fingerprints(&mut self) -> Result<(String, Vec<String>)> {
        let started = save_fingerprint_start();
        save_fingerprint_marker("begin", started.as_ref());
        let core = self
            .paths
            .iter()
            .filter(|p| !is_language_pack(p))
            .cloned()
            .collect::<Vec<_>>();
        if core.is_empty() {
            bail!("No PK3 archives found in {}", self.base.display());
        }
        save_fingerprint_marker("core-fingerprint:begin", started.as_ref());
        let identity = Self::fingerprint_entries(
            &mut self.packs,
            &self.core_entries,
            started.as_ref().map(|started| ("core", started)),
        )?;
        save_fingerprint_marker("core-fingerprint:ready", started.as_ref());
        save_fingerprint_marker("mounted-fingerprint:begin", started.as_ref());
        let mounted =
            self.fingerprint_with_progress(started.as_ref().map(|started| ("mounted", started)))?;
        save_fingerprint_marker("mounted-fingerprint:ready", started.as_ref());
        let mut legacy = vec![mounted];
        save_fingerprint_marker("language-discovery:begin", started.as_ref());
        let mut languages = self
            .paths
            .iter()
            .filter(|p| is_language_pack(p))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(managed) = self.base.ancestors().find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("game-data"))
        }) {
            if let Ok(entries) = fs::read_dir(managed.join("languages")) {
                languages.extend(
                    entries
                        .filter_map(|e| e.ok().map(|e| e.path()))
                        .filter(|p| is_language_pack(p)),
                );
            }
        }
        #[cfg(target_os = "horizon")]
        save_fingerprint_marker(
            &format!("language-discovery:count={}", languages.len()),
            started.as_ref(),
        );
        save_fingerprint_marker("language-discovery:ready", started.as_ref());
        let mut bases = vec![core.clone()];
        let mut variant_index = 0;
        save_fingerprint_marker("english-variants:begin", started.as_ref());
        for english in languages.iter().filter(|p| is_english_pack(p)) {
            let mut paths = core.clone();
            paths.push(english.clone());
            let stage = save_fingerprint_variant(variant_index);
            variant_index += 1;
            if let Some(stage) = stage {
                save_fingerprint_variant_marker(stage, "open:begin", started.as_ref());
            }
            // Includes the previous save identity that kept English in the core.
            if let Ok(mut mounted) = Self::open_variant_paths(&self.base, paths.clone()) {
                if let Some(stage) = stage {
                    save_fingerprint_variant_marker(stage, "open:ready", started.as_ref());
                    save_fingerprint_variant_marker(stage, "fingerprint:begin", started.as_ref());
                }
                if let Ok(fingerprint) = mounted.fingerprint_with_progress(
                    started
                        .as_ref()
                        .and_then(|started| stage.map(|stage| (stage, started))),
                ) {
                    if let Some(stage) = stage {
                        save_fingerprint_variant_marker(
                            stage,
                            "fingerprint:ready",
                            started.as_ref(),
                        );
                    }
                    legacy.push(fingerprint);
                    bases.push(paths);
                }
            }
        }
        save_fingerprint_marker("english-variants:ready", started.as_ref());
        save_fingerprint_marker("other-language-variants:begin", started.as_ref());
        for language in languages.iter().filter(|p| !is_english_pack(p)) {
            for base in &bases {
                let mut paths = base.clone();
                paths.push(language.clone());
                let stage = save_fingerprint_variant(variant_index);
                variant_index += 1;
                if let Some(stage) = stage {
                    save_fingerprint_variant_marker(stage, "open:begin", started.as_ref());
                }
                // An unreadable inactive cache must not prevent launching.
                if let Ok(mut mounted) = Self::open_variant_paths(&self.base, paths) {
                    if let Some(stage) = stage {
                        save_fingerprint_variant_marker(stage, "open:ready", started.as_ref());
                        save_fingerprint_variant_marker(
                            stage,
                            "fingerprint:begin",
                            started.as_ref(),
                        );
                    }
                    if let Ok(fingerprint) = mounted.fingerprint_with_progress(
                        started
                            .as_ref()
                            .and_then(|started| stage.map(|stage| (stage, started))),
                    ) {
                        if let Some(stage) = stage {
                            save_fingerprint_variant_marker(
                                stage,
                                "fingerprint:ready",
                                started.as_ref(),
                            );
                        }
                        legacy.push(fingerprint);
                    }
                }
            }
        }
        save_fingerprint_marker("other-language-variants:ready", started.as_ref());
        save_fingerprint_marker("deduplicate:begin", started.as_ref());
        legacy.sort();
        legacy.dedup();
        save_fingerprint_marker("deduplicate:ready", started.as_ref());
        save_fingerprint_marker("ready", started.as_ref());
        Ok((identity, legacy))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(&normalize(name))
    }
    pub fn pack_count(&self) -> usize {
        self.packs.len()
    }
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let &(p, i) = self
            .entries
            .get(&normalize(name))
            .with_context(|| format!("Asset not found: {name}"))?;
        let entry = self.packs[p].by_index(i)?;
        if entry.size() > MAX_ENTRY {
            bail!("Asset exceeds 128 MiB limit: {name}");
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry
            .take(MAX_ENTRY + 1)
            .read_to_end(&mut data)
            .with_context(|| format!("Reading {name}"))?;
        if data.len() as u64 > MAX_ENTRY {
            bail!("Expanded asset exceeds limit: {name}");
        }
        Ok(data)
    }
    pub fn maps(&self) -> Vec<String> {
        self.names()
            .filter(|n| n.starts_with("maps/") && n.ends_with(".bsp"))
            .map(|n| {
                n.trim_start_matches("maps/")
                    .trim_end_matches(".bsp")
                    .to_owned()
            })
            .collect()
    }
}
fn normalize(s: &str) -> String {
    s.replace('\\', "/").to_ascii_lowercase()
}

fn is_language_pack(path: &Path) -> bool {
    matches!(
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "pak4_english.pk3"
            | "pak4_german.pk3"
            | "pak4_french.pk3"
            | "pak4_spanish.pk3"
            | "pak4_dutch.pk3"
            | "base_g.pk3"
            | "base_f.pk3"
            | "base_s.pk3"
            | "base_d.pk3"
    )
}

fn is_english_pack(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("pak4_english.pk3"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn language_switch_preserves_save_identity_and_alphabetical_precedence() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("looking-glass-language-{}", std::process::id()));
        let base = root.join("game-data/base");
        let cache = root.join("game-data/languages");
        fs::create_dir_all(&base).context("create test base")?;
        fs::create_dir_all(&cache).context("create test cache")?;
        let write = |path: &Path, entry: &str, text: &[u8]| -> Result<()> {
            let mut z = zip::ZipWriter::new(File::create(path)?);
            z.start_file(entry, zip::write::FileOptions::default())?;
            z.write_all(text)?;
            z.finish()?;
            Ok(())
        };
        write(&base.join("pak0.pk3"), "ui/main.urc", b"English")?;
        let core_id = Assets::open(&base)?.fingerprint()?;
        write(&base.join("pak4_german.pk3"), "ui/main.urc", b"German")?;
        let mut german = Assets::open(&base)?;
        let old_german = german.fingerprint()?;
        assert_eq!(german.read("ui/main.urc")?, b"German");
        assert_eq!(german.save_fingerprints()?.0, core_id);
        drop(german);
        fs::copy(base.join("pak4_german.pk3"), cache.join("pak4_german.pk3"))
            .context("cache German test pack")?;
        fs::remove_file(base.join("pak4_german.pk3"))?;
        write(&base.join("pak4_french.pk3"), "ui/main.urc", b"French")?;
        let mut french = Assets::open(&base)?;
        let (id, legacy) = french.save_fingerprints()?;
        assert_eq!(id, core_id);
        assert!(legacy.contains(&old_german));
        assert_eq!(french.read("ui/main.urc")?, b"French");
        drop(french);
        write(&base.join("pak5_mod.pk3"), "ui/main.urc", b"Mod wins")?;
        let mut modified = Assets::open(&base)?;
        assert_eq!(modified.read("ui/main.urc")?, b"Mod wins");
        let (changed, legacy) = modified.save_fingerprints()?;
        assert_ne!(changed, core_id);
        assert!(!legacy.contains(&old_german));
        drop(modified);
        for path in [
            base.join("pak0.pk3"),
            base.join("pak4_french.pk3"),
            base.join("pak5_mod.pk3"),
            cache.join("pak4_german.pk3"),
        ] {
            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
        for path in [base, cache, root.join("game-data"), root] {
            fs::remove_dir(&path)
                .with_context(|| format!("remove directory {}", path.display()))?;
        }
        Ok(())
    }
    #[test]
    fn cached_core_index_preserves_core_override_when_language_pack_is_mounted() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("looking-glass-core-index-{}", std::process::id()));
        let base = root.join("game-data/base");
        fs::create_dir_all(&base)?;
        let write = |path: &Path, text: &[u8]| -> Result<()> {
            let mut z = zip::ZipWriter::new(File::create(path)?);
            z.start_file("UI\\main.urc", zip::write::FileOptions::default())?;
            z.write_all(text)?;
            z.finish()?;
            Ok(())
        };
        write(&base.join("pak0.pk3"), b"base")?;
        write(&base.join("pak4_english.pk3"), b"translation")?;
        write(&base.join("pak5_mod.pk3"), b"core override")?;

        let mut mounted = Assets::open(&base)?;
        let mut rebuilt_core = Assets::open_paths(
            &base,
            vec![base.join("pak0.pk3"), base.join("pak5_mod.pk3")],
        )?;
        assert_eq!(mounted.read("ui/main.urc")?, b"core override");
        assert_eq!(
            mounted.save_fingerprints()?.0,
            rebuilt_core.fingerprint()?,
            "cached core map must preserve sorted core-pack overrides while excluding translations"
        );
        drop(mounted);
        drop(rebuilt_core);
        for path in [
            base.join("pak0.pk3"),
            base.join("pak4_english.pk3"),
            base.join("pak5_mod.pk3"),
        ] {
            fs::remove_file(path)?;
        }
        fs::remove_dir(&base)?;
        fs::remove_dir(root.join("game-data"))?;
        fs::remove_dir(root)?;
        Ok(())
    }
    #[test]
    fn save_fingerprints_still_rejects_a_mount_with_only_language_archives() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "looking-glass-language-only-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        let path = root.join("pak4_english.pk3");
        let mut z = zip::ZipWriter::new(File::create(&path)?);
        z.start_file("ui/main.urc", zip::write::FileOptions::default())?;
        z.write_all(b"translation")?;
        z.finish()?;

        let mut mounted = Assets::open(&root)?;
        let error = mounted.save_fingerprints().unwrap_err().to_string();
        assert_eq!(
            error,
            format!("No PK3 archives found in {}", root.display())
        );
        drop(mounted);
        fs::remove_file(path)?;
        fs::remove_dir(root)?;
        Ok(())
    }
    #[test]
    fn core_identity_uses_open_mount_when_archive_path_is_replaced() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "looking-glass-core-replaced-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        let pack = root.join("pak0.pk3");
        let replacement = root.join("replacement.pk3");
        let backup = root.join("original.pk3");
        let write = |path: &Path, text: &[u8]| -> Result<()> {
            let mut z = zip::ZipWriter::new(File::create(path)?);
            z.start_file("ui/main.urc", zip::write::FileOptions::default())?;
            z.write_all(text)?;
            z.finish()?;
            Ok(())
        };
        write(&pack, b"mounted data")?;
        let mut mounted = Assets::open(&root)?;
        let mounted_id = mounted.fingerprint()?;

        write(&replacement, b"replacement data")?;
        fs::rename(&pack, &backup)?;
        fs::rename(&replacement, &pack)?;
        let cached_id = mounted.save_fingerprints()?.0;
        let current_id = Assets::open(&root)?.fingerprint()?;
        assert_eq!(
            cached_id, mounted_id,
            "save identity follows the active read-only mount"
        );
        assert_ne!(
            cached_id, current_id,
            "replacement files are outside the documented immutable-corpus contract"
        );

        drop(mounted);
        fs::remove_file(pack)?;
        fs::remove_file(backup)?;
        fs::remove_dir(root)?;
        Ok(())
    }
    #[test]
    fn open_still_rejects_malformed_archive_before_caching_core_index() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "looking-glass-malformed-pack-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        let path = root.join("pak0.pk3");
        fs::write(&path, b"not a zip archive")?;
        let error = Assets::open(&root)
            .err()
            .map(|error| error.to_string())
            .ok_or_else(|| anyhow::anyhow!("malformed archive unexpectedly opened"))?;
        assert!(
            error.contains("Invalid PK3"),
            "unexpected archive error: {error}"
        );
        fs::remove_file(path)?;
        fs::remove_dir(root)?;
        Ok(())
    }
    #[test]
    fn later_pack_wins_and_paths_are_case_insensitive() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("looking-glass-vfs-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        for (name, value) in [("pak0.pk3", b"old"), ("pak5.pk3", b"new")] {
            let mut z = zip::ZipWriter::new(File::create(dir.join(name))?);
            z.start_file("Textures/Test.ftx", zip::write::FileOptions::default())?;
            z.write_all(value)?;
            z.finish()?;
        }
        let mut a = Assets::open(&dir)?;
        assert_eq!(a.read("TEXTURES\\test.ftx")?, b"new");
        assert!(a.read("missing").is_err());
        drop(a);
        for name in ["pak0.pk3", "pak5.pk3"] {
            fs::remove_file(dir.join(name))?;
        }
        fs::remove_dir(dir)?;
        Ok(())
    }
}
