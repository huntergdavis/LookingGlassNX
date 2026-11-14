use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};
use zip::ZipArchive;

const MAX_ENTRY: u64 = 128 * 1024 * 1024;
#[cfg(target_os = "horizon")]
const HORIZON_ARCHIVE_TRACE_PACK_LIMIT: usize = 8;
#[cfg(target_os = "horizon")]
const HORIZON_ARCHIVE_TRACE_PROGRESS_LIMIT: usize = 8;

#[cfg(target_os = "horizon")]
fn horizon_archive_marker(stage: &str, ordinal: usize, scanned: usize, total: usize) {
    crate::horizon_diag(&format!(
        "rust:assets:{stage}:pack={ordinal}:entries={scanned}/{total}"
    ));
}

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

fn archive_sort_key(path: &Path) -> OsString {
    path.file_name().unwrap_or_default().to_ascii_lowercase()
}

fn stable_variant_ranks(
    core_paths: &[(usize, PathBuf)],
    english_path: &Path,
    pack_count: usize,
) -> (Vec<usize>, usize) {
    let mut order = core_paths
        .iter()
        .map(|(index, path)| (Some(*index), path.clone()))
        .collect::<Vec<_>>();
    order.push((None, english_path.to_path_buf()));
    order.sort_by_key(|(_, path)| archive_sort_key(path));

    let mut core_rank = vec![usize::MAX; pack_count];
    let mut english_rank = None;
    for (rank, (pack_index, _)) in order.into_iter().enumerate() {
        if let Some(pack_index) = pack_index {
            core_rank[pack_index] = rank;
        } else {
            english_rank = Some(rank);
        }
    }
    (
        core_rank,
        english_rank.expect("English path is appended to variant order"),
    )
}

fn merge_variant_entries(
    mut core_entries: BTreeMap<String, (usize, usize)>,
    english_entries: BTreeMap<String, (usize, usize)>,
    english_pack_index: usize,
    core_rank: &[usize],
    english_rank: usize,
) -> BTreeMap<String, (usize, usize)> {
    for (name, (_, index)) in english_entries {
        let english_wins = match core_entries.get(&name) {
            None => true,
            Some(&(core_pack, _)) => english_rank > core_rank[core_pack],
        };
        if english_wins {
            core_entries.insert(name, (english_pack_index, index));
        }
    }
    core_entries
}

fn record_english_variant(
    legacy: &mut Vec<String>,
    bases: &mut Vec<Vec<PathBuf>>,
    paths: Vec<PathBuf>,
    fingerprint: Result<String>,
) {
    if let Ok(fingerprint) = fingerprint {
        legacy.push(fingerprint);
        // Keep successful bases even when the identity later deduplicates.
        bases.push(paths);
    }
}

/// Read-only PK3 mount. Later alphabetically sorted packs override earlier packs.
/// Archive paths, contents, and read permissions must remain stable while this
/// mount is in use; save identities reuse the metadata and handles captured by
/// this mount.
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

    fn entries_for_pack(
        pack: &mut ZipArchive<File>,
        pack_index: usize,
    ) -> Result<BTreeMap<String, (usize, usize)>> {
        let mut entries = BTreeMap::new();
        for index in 0..pack.len() {
            let entry = pack.by_index(index)?;
            if !entry.is_dir() {
                entries.insert(normalize(entry.name()), (pack_index, index));
            }
        }
        Ok(entries)
    }

    fn mounted_english_variant_entries(
        &mut self,
        english_pack_index: usize,
    ) -> Result<BTreeMap<String, (usize, usize)>> {
        let english_path = self
            .paths
            .get(english_pack_index)
            .context("English PK3 is not in the active mount")?
            .clone();

        // Reproduce open_variant_paths' exact input order and stable sort. The
        // English archive is appended after core paths, so it wins equal keys.
        let core_paths = self
            .paths
            .iter()
            .enumerate()
            .filter(|(_, path)| !is_language_pack(path))
            .map(|(index, path)| (index, path.clone()))
            .collect::<Vec<_>>();
        let (core_rank, english_rank) =
            stable_variant_ranks(&core_paths, &english_path, self.paths.len());

        let english_entries = Self::entries_for_pack(
            self.packs
                .get_mut(english_pack_index)
                .context("English PK3 missing from active archive handles")?,
            english_pack_index,
        )?;
        Ok(merge_variant_entries(
            self.core_entries.clone(),
            english_entries,
            english_pack_index,
            &core_rank,
            english_rank,
        ))
    }

    pub fn open(base: &Path) -> Result<Self> {
        #[cfg(target_os = "horizon")]
        crate::horizon_diag("rust:assets:archive-discovery-begin");
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
        #[cfg(target_os = "horizon")]
        crate::horizon_diag(&format!("rust:assets:archive-discovery-ready:packs={}", paths.len()));
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
        paths.sort_by_key(|path| archive_sort_key(path));
        if paths.is_empty() {
            bail!("No PK3 archives found in {}", base.display());
        }
        let mut entries = BTreeMap::new();
        let mut core_entries = BTreeMap::new();
        let mut packs = Vec::new();
        for path in &paths {
            #[cfg(target_os = "horizon")]
            let ordinal = packs.len() + 1;
            #[cfg(target_os = "horizon")]
            let trace_pack = cache_core_index && packs.len() < HORIZON_ARCHIVE_TRACE_PACK_LIMIT;
            #[cfg(target_os = "horizon")]
            if trace_pack {
                horizon_archive_marker("pack-open-begin", ordinal, 0, 0);
            }
            let archive_file = match File::open(&path) {
                Ok(file) => {
                    #[cfg(target_os = "horizon")]
                    if trace_pack {
                        horizon_archive_marker("pack-file-open-ready", ordinal, 0, 0);
                        horizon_archive_marker("pack-zip-parse-begin", ordinal, 0, 0);
                    }
                    file
                }
                Err(error) => {
                    #[cfg(target_os = "horizon")]
                    if trace_pack {
                        horizon_archive_marker("pack-file-open-error", ordinal, 0, 0);
                    }
                    return Err(error.into());
                }
            };
            let mut pack = match ZipArchive::new(archive_file) {
                Ok(pack) => pack,
                Err(error) => {
                    #[cfg(target_os = "horizon")]
                    if trace_pack {
                        horizon_archive_marker("pack-zip-parse-error", ordinal, 0, 0);
                    }
                    return Err(error).with_context(|| format!("Invalid PK3: {}", path.display()));
                }
            };
            let entry_count = pack.len();
            #[cfg(target_os = "horizon")]
            if trace_pack {
                horizon_archive_marker("pack-open-ready", ordinal, entry_count, entry_count);
            }
            #[cfg(target_os = "horizon")]
            let progress_step = entry_count.saturating_add(HORIZON_ARCHIVE_TRACE_PROGRESS_LIMIT - 1)
                / HORIZON_ARCHIVE_TRACE_PROGRESS_LIMIT;
            let p = packs.len();
            let is_core = !is_language_pack(path);
            for i in 0..entry_count {
                let entry = pack.by_index(i)?;
                if !entry.is_dir() {
                    let name = normalize(entry.name());
                    entries.insert(name.clone(), (p, i));
                    if is_core && cache_core_index {
                        core_entries.insert(name, (p, i));
                    }
                }
                #[cfg(target_os = "horizon")]
                {
                    let scanned = i + 1;
                    if trace_pack
                        && (scanned % progress_step == 0 || scanned == entry_count)
                    {
                        horizon_archive_marker("pack-index-progress", ordinal, scanned, entry_count);
                    }
                }
            }
            #[cfg(target_os = "horizon")]
            if trace_pack {
                horizon_archive_marker("pack-index-ready", ordinal, entry_count, entry_count);
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
            let mounted_pack = self.paths.iter().position(|path| path == english);
            let fingerprint = if let Some(pack_index) = mounted_pack {
                match self.mounted_english_variant_entries(pack_index) {
                    Ok(entries) => {
                        if let Some(stage) = stage {
                            save_fingerprint_variant_marker(stage, "open:ready", started.as_ref());
                            save_fingerprint_variant_marker(
                                stage,
                                "fingerprint:begin",
                                started.as_ref(),
                            );
                        }
                        Self::fingerprint_entries(
                            &mut self.packs,
                            &entries,
                            started
                                .as_ref()
                                .and_then(|started| stage.map(|stage| (stage, started))),
                        )
                    }
                    Err(error) => Err(error),
                }
            } else {
                match Self::open_variant_paths(&self.base, paths.clone()) {
                    Ok(mut mounted) => {
                        if let Some(stage) = stage {
                            save_fingerprint_variant_marker(stage, "open:ready", started.as_ref());
                            save_fingerprint_variant_marker(
                                stage,
                                "fingerprint:begin",
                                started.as_ref(),
                            );
                        }
                        mounted.fingerprint_with_progress(
                            started
                                .as_ref()
                                .and_then(|started| stage.map(|stage| (stage, started))),
                        )
                    }
                    Err(error) => Err(error),
                }
            };
            if fingerprint.is_ok() {
                if let Some(stage) = stage {
                    save_fingerprint_variant_marker(stage, "fingerprint:ready", started.as_ref());
                }
            }
            record_english_variant(&mut legacy, &mut bases, paths, fingerprint);
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

    fn write_pack(path: &Path, entries: &[(&str, &[u8])]) -> Result<()> {
        let mut archive = zip::ZipWriter::new(File::create(path)?);
        for (name, contents) in entries {
            archive.start_file(*name, zip::write::FileOptions::default())?;
            archive.write_all(contents)?;
        }
        archive.finish()?;
        Ok(())
    }

    fn fixture_root(name: &str) -> Result<PathBuf> {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-fixtures");
        fs::create_dir_all(&parent)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root = parent.join(format!("{name}-{}-{nonce}", std::process::id()));
        // Exclusive creation makes cleanup ownership unambiguous, even after
        // stale fixtures or PID reuse from an earlier interrupted run.
        fs::create_dir(&root)?;
        Ok(root)
    }

    // Retained full-reopen oracle for the pre-optimization legacy algorithm.
    fn reference_save_fingerprints(assets: &mut Assets) -> Result<(String, Vec<String>)> {
        let core = assets
            .paths
            .iter()
            .filter(|path| !is_language_pack(path))
            .cloned()
            .collect::<Vec<_>>();
        if core.is_empty() {
            bail!("No PK3 archives found in {}", assets.base.display());
        }
        let mut core_mount = Assets::open_variant_paths(&assets.base, core.clone())?;
        let core_identity = core_mount.fingerprint()?;
        let mut legacy = vec![assets.fingerprint()?];
        let mut languages = assets
            .paths
            .iter()
            .filter(|path| is_language_pack(path))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(managed) = assets.base.ancestors().find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("game-data"))
        }) {
            if let Ok(entries) = fs::read_dir(managed.join("languages")) {
                languages.extend(
                    entries
                        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                        .filter(|path| is_language_pack(path)),
                );
            }
        }

        let mut bases = vec![core.clone()];
        for english in languages.iter().filter(|path| is_english_pack(path)) {
            let mut paths = core.clone();
            paths.push(english.clone());
            if let Ok(mut mounted) = Assets::open_variant_paths(&assets.base, paths.clone()) {
                if let Ok(fingerprint) = mounted.fingerprint() {
                    legacy.push(fingerprint);
                    bases.push(paths);
                }
            }
        }
        for language in languages.iter().filter(|path| !is_english_pack(path)) {
            for base in &bases {
                let mut paths = base.clone();
                paths.push(language.clone());
                if let Ok(mut mounted) = Assets::open_variant_paths(&assets.base, paths) {
                    if let Ok(fingerprint) = mounted.fingerprint() {
                        legacy.push(fingerprint);
                    }
                }
            }
        }
        legacy.sort();
        legacy.dedup();
        Ok((core_identity, legacy))
    }

    #[test]
    fn english_variant_rank_preserves_stable_sort_ties_and_later_core() {
        // Synthetic equal-key paths pin the helper's tie rule independently
        // of is_language_pack's current filename classification.
        let core_path = PathBuf::from("PAK4_ENGLISH.PK3");
        let english_path = PathBuf::from("pak4_english.pk3");
        let (ranks, english_rank) = stable_variant_ranks(&[(0, core_path)], &english_path, 2);
        assert!(
            english_rank > ranks[0],
            "appended English wins an equal key"
        );
        let merged = merge_variant_entries(
            BTreeMap::from([("ui/name".to_owned(), (0, 3))]),
            BTreeMap::from([("ui/name".to_owned(), (1, 4))]),
            1,
            &ranks,
            english_rank,
        );
        assert_eq!(merged["ui/name"], (1, 4));

        let later_core = PathBuf::from("pak5_mod.pk3");
        let (ranks, english_rank) = stable_variant_ranks(&[(0, later_core)], &english_path, 2);
        assert!(ranks[0] > english_rank);
        let merged = merge_variant_entries(
            BTreeMap::from([("ui/name".to_owned(), (0, 3))]),
            BTreeMap::from([("ui/name".to_owned(), (1, 4))]),
            1,
            &ranks,
            english_rank,
        );
        assert_eq!(merged["ui/name"], (0, 3));
    }

    #[test]
    fn mounted_english_reuse_matches_full_reopen_legacy_vector() -> Result<()> {
        let root = fixture_root("english-reuse")?;
        let game_data = root.join("game-data");
        let base = game_data.join("base");
        let languages = game_data.join("languages");
        fs::create_dir_all(&base)?;
        fs::create_dir_all(&languages)?;
        write_pack(
            &base.join("pak0.pk3"),
            &[
                ("UI\\early.urc", b"core early"),
                ("UI\\duplicate.urc", b"duplicate first"),
                ("ui/duplicate.urc", b"duplicate last"),
            ],
        )?;
        write_pack(
            &base.join("pak4_english.pk3"),
            &[
                ("ui/early.urc", b"English override"),
                ("ui/late.urc", b"English late"),
                ("ui/german.urc", b"English German"),
                ("UI\\duplicate-english.urc", b"English duplicate first"),
                ("ui/duplicate-english.urc", b"English duplicate last"),
            ],
        )?;
        write_pack(
            &base.join("pak4_german.pk3"),
            &[("ui/german.urc", b"German active"), ("lang/german", b"de")],
        )?;
        write_pack(
            &base.join("pak5_mod.pk3"),
            &[("ui/late.urc", b"later core wins")],
        )?;
        // Same basename as the mounted English pack, but a different path;
        // this one must stay on the old external-pack fallback.
        write_pack(
            &languages.join("pak4_english.pk3"),
            &[("ui/external.urc", b"external English")],
        )?;
        write_pack(
            &languages.join("pak4_french.pk3"),
            &[("ui/external.urc", b"French"), ("lang/french", b"fr")],
        )?;

        let mut actual_assets = Assets::open(&base)?;
        let names_before = actual_assets.names().map(str::to_owned).collect::<Vec<_>>();
        let packs_before = actual_assets.pack_count();
        assert_eq!(actual_assets.read("ui/early.urc")?, b"English override");
        assert_eq!(actual_assets.read("ui/late.urc")?, b"later core wins");
        assert_eq!(actual_assets.read("ui/german.urc")?, b"German active");
        assert_eq!(actual_assets.read("ui/duplicate.urc")?, b"duplicate last");
        assert_eq!(
            actual_assets.read("ui/duplicate-english.urc")?,
            b"English duplicate last"
        );

        let actual = actual_assets.save_fingerprints()?;
        assert_eq!(
            actual_assets.save_fingerprints()?,
            actual,
            "repeated fingerprint calls preserve results and mount state"
        );
        let names_after = actual_assets.names().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(names_after, names_before);
        assert_eq!(actual_assets.pack_count(), packs_before);
        assert_eq!(actual_assets.read("ui/early.urc")?, b"English override");
        assert_eq!(actual_assets.read("ui/late.urc")?, b"later core wins");
        assert_eq!(actual_assets.read("ui/german.urc")?, b"German active");
        assert_eq!(actual_assets.read("ui/duplicate.urc")?, b"duplicate last");
        assert_eq!(
            actual_assets.read("ui/duplicate-english.urc")?,
            b"English duplicate last"
        );

        let mut reference_assets = Assets::open(&base)?;
        let reference = reference_save_fingerprints(&mut reference_assets)?;
        assert_eq!(
            actual, reference,
            "all legacy identities match the full-reopen oracle"
        );

        drop(actual_assets);
        drop(reference_assets);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn deduplicated_english_identity_seeds_distinct_downstream_language() -> Result<()> {
        let root = fixture_root("english-dedup-language")?;
        let game_data = root.join("game-data");
        let base = game_data.join("base");
        let languages = game_data.join("languages");
        fs::create_dir_all(&base)?;
        fs::create_dir_all(&languages)?;
        let core_path = base.join("pak3_core.pk3");
        let english_path = base.join("pak4_english.pk3");
        let dutch_path = languages.join("pak4_dutch.pk3");
        write_pack(&core_path, &[("ui/shared.urc", b"same baseline")])?;
        write_pack(&english_path, &[("ui/shared.urc", b"same baseline")])?;
        write_pack(
            &dutch_path,
            &[
                ("ui/shared.urc", b"Dutch override"),
                ("ui/dutch-only.urc", b"Dutch addition"),
            ],
        )?;

        let core_identity =
            Assets::open_variant_paths(&base, vec![core_path.clone()])?.fingerprint()?;
        let english_identity =
            Assets::open_variant_paths(&base, vec![core_path.clone(), english_path.clone()])?
                .fingerprint()?;
        assert_eq!(core_identity, english_identity);

        // pak4_dutch sorts before pak4_english. In the old full-reopen
        // composition, English therefore overrides Dutch in this combination,
        // making it distinct from core + Dutch despite the duplicate English
        // baseline identity.
        let dutch_only =
            Assets::open_variant_paths(&base, vec![core_path.clone(), dutch_path.clone()])?
                .fingerprint()?;
        let english_then_dutch = Assets::open_variant_paths(
            &base,
            vec![core_path.clone(), english_path.clone(), dutch_path.clone()],
        )?
        .fingerprint()?;
        assert_ne!(dutch_only, english_then_dutch);
        assert_ne!(core_identity, english_then_dutch);

        let mut assets = Assets::open(&base)?;
        let actual = assets.save_fingerprints()?;
        let mut reference_assets = Assets::open(&base)?;
        let reference = reference_save_fingerprints(&mut reference_assets)?;
        assert_eq!(
            actual, reference,
            "the full legacy vector retains both variants"
        );
        assert!(actual.1.contains(&dutch_only));
        assert!(actual.1.contains(&english_then_dutch));

        drop(assets);
        drop(reference_assets);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    // Coverage boundary: with stable readable archives, production maps come
    // from successfully enumerated active ZipArchive handles. A mounted-only
    // mid-fingerprint error needs an artificial fault seam; this test pins the
    // failure-record skip contract, but does not claim that end-to-end path.
    fn failed_english_variant_is_skipped_without_adding_a_base() -> Result<()> {
        let root = fixture_root("english-failure")?;
        let pack_path = root.join("pak0.pk3");
        write_pack(&pack_path, &[("ui/name", b"core")])?;
        let mut mounted = Assets::open(&root)?;
        let invalid = BTreeMap::from([("ui/name".to_owned(), (0, usize::MAX))]);
        let failed = Assets::fingerprint_entries(&mut mounted.packs, &invalid, None);
        assert!(
            failed.is_err(),
            "invalid entry index injects fingerprint failure"
        );

        let mut legacy = vec!["mounted".to_owned()];
        let mut bases = vec![vec![pack_path.clone()]];
        record_english_variant(
            &mut legacy,
            &mut bases,
            vec![pack_path.clone(), root.join("pak4_english.pk3")],
            failed,
        );
        assert_eq!(legacy, vec!["mounted".to_owned()]);
        assert_eq!(bases, vec![vec![pack_path.clone()]]);
        drop(mounted);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn duplicate_english_identity_still_adds_a_language_base() {
        let core = PathBuf::from("pak0.pk3");
        let english = PathBuf::from("pak4_english.pk3");
        let mut legacy = vec!["same-identity".to_owned()];
        let mut bases = vec![vec![core.clone()]];

        record_english_variant(
            &mut legacy,
            &mut bases,
            vec![core.clone(), english.clone()],
            Ok("same-identity".to_owned()),
        );
        legacy.sort();
        legacy.dedup();

        assert_eq!(legacy, vec!["same-identity".to_owned()]);
        assert_eq!(bases, vec![vec![core.clone()], vec![core, english]]);
    }

    #[test]
    fn invalid_external_english_variant_keeps_skip_behavior() -> Result<()> {
        let root = fixture_root("english-open-error")?;
        let game_data = root.join("game-data");
        let base = game_data.join("base");
        let languages = game_data.join("languages");
        fs::create_dir_all(&base)?;
        fs::create_dir_all(&languages)?;
        write_pack(&base.join("pak0.pk3"), &[("ui/core", b"core")])?;
        fs::write(languages.join("pak4_english.pk3"), b"not a zip archive")?;

        let mut actual_assets = Assets::open(&base)?;
        let actual = actual_assets.save_fingerprints()?;
        let mut reference_assets = Assets::open(&base)?;
        let reference = reference_save_fingerprints(&mut reference_assets)?;
        assert_eq!(
            actual, reference,
            "unreadable external English pack is skipped"
        );

        drop(actual_assets);
        drop(reference_assets);
        fs::remove_dir_all(root)?;
        Ok(())
    }

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
