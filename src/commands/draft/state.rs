use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::cli::CliError;

const STATE_VERSION: u32 = 1;
const STATE_DIRECTORY: &str = "wechat-oa-cli/drafts";
const MAX_STATE_FILE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct DraftStateLock {
    _file: File,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum LegacyStatePolicy {
    Reject,
    Adopt,
    Discard,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DraftState {
    version: u32,
    #[serde(default)]
    assets: BTreeMap<String, AssetCache>,
    #[serde(default)]
    pending_push: Option<String>,
    #[serde(default)]
    completed_pushes: BTreeMap<String, String>,
    #[serde(skip)]
    path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AssetCache {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    permanent_image: Option<PermanentImageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    article_image: Option<ArticleImageRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PermanentImageRecord {
    sha256: String,
    media_id: String,
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ArticleImageRecord {
    sha256: String,
    url: String,
}

impl DraftState {
    pub(crate) async fn acquire(
        root: &Path,
        account_id: &str,
        legacy_policy: LegacyStatePolicy,
    ) -> Result<(DraftStateLock, Self), CliError> {
        // This is also the namespace used by pre-account-scoping releases. Holding
        // its lock serializes migration between accounts and with those releases.
        let accountless_namespace =
            format!("draft-{}", bytes_sha256(root.to_string_lossy().as_bytes()));
        let legacy_directory = state_directory(&accountless_namespace)?;
        let _migration_lock = acquire_directory_lock(&legacy_directory).await?;

        let identity = format!("{}\0{account_id}", root.to_string_lossy());
        let namespace = format!("draft-{}", bytes_sha256(identity.as_bytes()));
        let (state_lock, mut state) = Self::acquire_namespace(&namespace).await?;
        state
            .handle_legacy(root, &legacy_directory, legacy_policy)
            .await?;
        Ok((state_lock, state))
    }

    pub(crate) async fn acquire_operation(
        operation: &str,
        account_id: &str,
    ) -> Result<(DraftStateLock, Self), CliError> {
        let identity = format!("{operation}\0{account_id}");
        let namespace = format!("operation-{}", bytes_sha256(identity.as_bytes()));
        Self::acquire_namespace(&namespace).await
    }

    async fn acquire_namespace(namespace: &str) -> Result<(DraftStateLock, Self), CliError> {
        let directory = state_directory(namespace)?;
        let state_lock = acquire_directory_lock(&directory).await?;
        let path = directory.join("state.json");
        let state = Self::load_path(path).await?;
        Ok((state_lock, state))
    }

    async fn load_path(path: PathBuf) -> Result<Self, CliError> {
        recover_backup(&path).await?;
        match tokio::fs::File::open(&path).await {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take((MAX_STATE_FILE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|source| CliError::ReadFile {
                        path: path.clone(),
                        source,
                    })?;
                if bytes.len() > MAX_STATE_FILE_BYTES {
                    return Err(CliError::LocalState(format!(
                        "state file {} exceeds the 16 MiB limit",
                        path.display()
                    )));
                }
                let mut state: Self =
                    serde_json::from_slice(&bytes).map_err(|source| CliError::DecodeInput {
                        source_name: path.display().to_string(),
                        source,
                    })?;
                if state.version != STATE_VERSION {
                    return Err(CliError::InvalidDraft(format!(
                        "unsupported state version {} in {}",
                        state.version,
                        path.display()
                    )));
                }
                state.path = path;
                Ok(state)
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                version: STATE_VERSION,
                assets: BTreeMap::new(),
                pending_push: None,
                completed_pushes: BTreeMap::new(),
                path,
            }),
            Err(source) => Err(CliError::ReadFile { path, source }),
        }
    }

    async fn handle_legacy(
        &mut self,
        root: &Path,
        legacy_directory: &Path,
        policy: LegacyStatePolicy,
    ) -> Result<(), CliError> {
        let accountless_path = legacy_directory.join("state.json");
        let local_path = root.join(".wechat-oa/state.json");
        let mut legacy_paths = Vec::new();
        for legacy_path in [accountless_path, local_path] {
            recover_backup(&legacy_path).await?;
            if tokio::fs::try_exists(&legacy_path)
                .await
                .map_err(|source| CliError::ReadFile {
                    path: legacy_path.clone(),
                    source,
                })?
            {
                legacy_paths.push(legacy_path);
            }
        }

        if legacy_paths.is_empty() {
            return Ok(());
        }

        match policy {
            LegacyStatePolicy::Reject => Err(CliError::LegacyStatePolicyRequired),
            LegacyStatePolicy::Adopt => {
                let current_exists = tokio::fs::try_exists(&self.path).await.map_err(|source| {
                    CliError::ReadFile {
                        path: self.path.clone(),
                        source,
                    }
                })?;
                if current_exists || legacy_paths.len() != 1 {
                    return Err(CliError::ConflictingLegacyStates);
                }
                self.migrate_from(legacy_paths.remove(0)).await
            }
            LegacyStatePolicy::Discard => {
                for legacy_path in legacy_paths {
                    remove_legacy_file(&legacy_path).await?;
                }
                Ok(())
            }
        }
    }

    async fn migrate_from(&mut self, legacy_path: PathBuf) -> Result<(), CliError> {
        let mut legacy = Self::load_path(legacy_path.clone()).await?;
        legacy.path = self.path.clone();
        legacy.save().await?;
        *self = legacy;
        remove_legacy_file(&legacy_path).await
    }

    pub(crate) fn permanent_image(&self, key: &str, sha256: &str) -> Option<(&str, &str)> {
        match self
            .assets
            .get(key)
            .and_then(|asset| asset.permanent_image.as_ref())
        {
            Some(PermanentImageRecord {
                sha256: cached_hash,
                media_id,
                url,
            }) if cached_hash == sha256 => Some((media_id, url)),
            _ => None,
        }
    }

    pub(crate) fn article_image(&self, key: &str, sha256: &str) -> Option<&str> {
        match self
            .assets
            .get(key)
            .and_then(|asset| asset.article_image.as_ref())
        {
            Some(ArticleImageRecord {
                sha256: cached_hash,
                url,
            }) if cached_hash == sha256 => Some(url),
            _ => None,
        }
    }

    pub(crate) fn record_permanent_image(
        &mut self,
        key: String,
        sha256: String,
        media_id: String,
        url: String,
    ) {
        self.assets.entry(key).or_default().permanent_image = Some(PermanentImageRecord {
            sha256,
            media_id,
            url,
        });
    }

    pub(crate) fn record_article_image(&mut self, key: String, sha256: String, url: String) {
        self.assets.entry(key).or_default().article_image =
            Some(ArticleImageRecord { sha256, url });
    }

    pub(crate) fn completed_media_id(&self, fingerprint: &str) -> Option<&str> {
        self.completed_pushes.get(fingerprint).map(String::as_str)
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pending_push.is_some()
    }

    #[cfg(test)]
    pub(crate) fn test_directories(
        root: &Path,
        account_id: &str,
    ) -> Result<[PathBuf; 2], CliError> {
        let accountless_namespace =
            format!("draft-{}", bytes_sha256(root.to_string_lossy().as_bytes()));
        let identity = format!("{}\0{account_id}", root.to_string_lossy());
        let account_namespace = format!("draft-{}", bytes_sha256(identity.as_bytes()));
        Ok([
            state_directory(&accountless_namespace)?,
            state_directory(&account_namespace)?,
        ])
    }

    pub(crate) fn begin_push(&mut self, fingerprint: String) {
        self.pending_push = Some(fingerprint);
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending_push = None;
    }

    pub(crate) fn complete_push(&mut self, fingerprint: String, media_id: String) {
        self.pending_push = None;
        self.completed_pushes.insert(fingerprint, media_id);
    }

    pub(crate) async fn save(&self) -> Result<(), CliError> {
        let parent = self.path.parent().ok_or_else(|| {
            CliError::InvalidDraft("state file has no parent directory".to_owned())
        })?;
        create_private_dir(parent).await?;
        let bytes = serde_json::to_vec_pretty(self)?;
        let temporary = self
            .path
            .with_extension(format!("json.tmp-{}", std::process::id()));
        let target = self.path.clone();
        tokio::task::spawn_blocking(move || write_private_atomic(&temporary, &target, &bytes))
            .await
            .map_err(|source| CliError::LocalState(format!("state write task failed: {source}")))?
            .map_err(|source| CliError::WriteFile {
                path: self.path.clone(),
                source,
            })
    }
}

fn state_directory(namespace: &str) -> Result<PathBuf, CliError> {
    let base_dirs = BaseDirs::new().ok_or_else(|| {
        CliError::LocalState("cannot determine the user data directory".to_owned())
    })?;
    Ok(base_dirs
        .data_local_dir()
        .join(STATE_DIRECTORY)
        .join(namespace))
}

async fn acquire_directory_lock(directory: &Path) -> Result<DraftStateLock, CliError> {
    create_private_dir(directory).await?;
    let lock_path = directory.join("state.lock");
    let lock_file = tokio::task::spawn_blocking({
        let lock_path = lock_path.clone();
        move || open_and_lock(&lock_path)
    })
    .await
    .map_err(|source| CliError::LocalState(format!("state lock task failed: {source}")))?
    .map_err(|source| CliError::WriteFile {
        path: lock_path,
        source,
    })?;
    Ok(DraftStateLock { _file: lock_file })
}

async fn remove_legacy_file(path: &Path) -> Result<(), CliError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(CliError::WriteFile {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    let backup = path.with_extension("json.backup");
    match tokio::fs::remove_file(&backup).await {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(CliError::WriteFile {
                path: backup,
                source,
            });
        }
    }

    if let Some(parent) = path.parent() {
        match tokio::fs::remove_dir(parent).await {
            Ok(()) => {}
            Err(source)
                if matches!(
                    source.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                ) => {}
            Err(source) => {
                return Err(CliError::WriteFile {
                    path: parent.to_path_buf(),
                    source,
                });
            }
        }
    }
    Ok(())
}

pub(crate) async fn file_sha256(path: &Path) -> Result<String, CliError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|source| CliError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|source| CliError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest_hex(digest.finalize()))
}

pub(crate) fn bytes_sha256(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest_hex(Sha256::digest(bytes))
}

fn digest_hex(digest: impl AsRef<[u8]>) -> String {
    let digest = digest.as_ref();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

async fn create_private_dir(path: &Path) -> Result<(), CliError> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|source| CliError::WriteFile {
            path: path.to_path_buf(),
            source,
        })?;
    set_private_dir_permissions(path).map_err(|source| CliError::WriteFile {
        path: path.to_path_buf(),
        source,
    })
}

fn open_and_lock(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    set_private_file_mode(&mut options);
    let file = options.open(path)?;
    set_private_file_permissions(path)?;
    lock_exclusive(&file)?;
    Ok(file)
}

fn write_private_atomic(temporary: &Path, target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    set_private_file_mode(&mut options);
    let mut file = options.open(temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    replace_file(temporary, target)?;
    set_private_file_permissions(target)?;
    if let Some(parent) = target.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

async fn recover_backup(target: &Path) -> Result<(), CliError> {
    let backup = target.with_extension("json.backup");
    let target_exists =
        tokio::fs::try_exists(target)
            .await
            .map_err(|source| CliError::ReadFile {
                path: target.to_path_buf(),
                source,
            })?;
    if target_exists {
        return Ok(());
    }
    let backup_exists =
        tokio::fs::try_exists(&backup)
            .await
            .map_err(|source| CliError::ReadFile {
                path: backup.clone(),
                source,
            })?;
    if backup_exists {
        tokio::fs::rename(&backup, target)
            .await
            .map_err(|source| CliError::WriteFile {
                path: target.to_path_buf(),
                source,
            })?;
    }
    Ok(())
}

#[cfg(unix)]
fn replace_file(temporary: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(temporary, target)
}

#[cfg(windows)]
fn replace_file(temporary: &Path, target: &Path) -> std::io::Result<()> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut c_void,
            reserved: *mut c_void,
        ) -> i32;
    }

    if !target.exists() {
        return fs::rename(temporary, target);
    }

    let backup = target.with_extension("json.backup");
    let _ = fs::remove_file(&backup);
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let target_wide = wide(target);
    let temporary_wide = wide(temporary);
    let backup_wide = wide(&backup);
    // SAFETY: all paths are NUL-terminated UTF-16 buffers that remain alive
    // for the duration of this synchronous call.
    let result = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            temporary_wide.as_ptr(),
            backup_wide.as_ptr(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        let _ = fs::remove_file(backup);
        Ok(())
    }
}

#[cfg(unix)]
fn set_private_file_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_file_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn lock_exclusive(file: &File) -> std::io::Result<()> {
    use std::os::{raw::c_int, unix::io::AsRawFd};

    const LOCK_EX: c_int = 2;
    unsafe extern "C" {
        fn flock(fd: c_int, operation: c_int) -> c_int;
    }

    loop {
        // SAFETY: the file descriptor remains valid for this call and flock
        // does not retain a pointer into Rust memory.
        if unsafe { flock(file.as_raw_fd(), LOCK_EX) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(windows)]
fn lock_exclusive(file: &File) -> std::io::Result<()> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle, ptr};

    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        event: *mut c_void,
    }

    unsafe extern "system" {
        fn LockFileEx(
            file: *mut c_void,
            flags: u32,
            reserved: u32,
            bytes_low: u32,
            bytes_high: u32,
            overlapped: *mut Overlapped,
        ) -> i32;
    }

    const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x0000_0002;
    let mut overlapped = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: 0,
        offset_high: 0,
        event: ptr::null_mut(),
    };
    // SAFETY: the OS handle and OVERLAPPED pointer are valid for the duration
    // of the synchronous call. Closing the file releases the lock.
    let result = unsafe {
        LockFileEx(
            file.as_raw_handle().cast(),
            LOCKFILE_EXCLUSIVE_LOCK,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn empty_state() -> DraftState {
        DraftState {
            version: STATE_VERSION,
            assets: BTreeMap::new(),
            pending_push: None,
            completed_pushes: BTreeMap::new(),
            path: PathBuf::from("state.json"),
        }
    }

    fn temporary_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("wechat-oa-{label}-{}-{nonce}", std::process::id()))
    }

    fn write_state(path: &Path, pending: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut state = empty_state();
        state.pending_push = Some(pending.to_owned());
        fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
    }

    #[test]
    fn keeps_permanent_and_article_records_for_the_same_file() {
        let mut state = empty_state();
        state.record_permanent_image(
            "assets/image.png".to_owned(),
            "hash".to_owned(),
            "media-id".to_owned(),
            "permanent-url".to_owned(),
        );
        state.record_article_image(
            "assets/image.png".to_owned(),
            "hash".to_owned(),
            "article-url".to_owned(),
        );

        assert_eq!(
            state.permanent_image("assets/image.png", "hash"),
            Some(("media-id", "permanent-url"))
        );
        assert_eq!(
            state.article_image("assets/image.png", "hash"),
            Some("article-url")
        );
    }

    #[test]
    fn sha256_is_stable() {
        assert_eq!(
            bytes_sha256(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[tokio::test]
    async fn state_can_be_replaced_and_is_private() {
        let account = format!("test-account-{}", std::process::id());
        let (state_lock, mut state) = DraftState::acquire_operation("state-test", &account)
            .await
            .unwrap();
        let directory = state.path.parent().unwrap().to_path_buf();
        state.begin_push("fingerprint".to_owned());
        state.save().await.unwrap();
        state.complete_push("fingerprint".to_owned(), "media-id".to_owned());
        state.save().await.unwrap();

        let loaded = DraftState::load_path(state.path.clone()).await.unwrap();
        assert_eq!(loaded.completed_media_id("fingerprint"), Some("media-id"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&state.path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        drop(state_lock);
        let _ = fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn draft_state_is_isolated_by_account() {
        let root = temporary_root("state-account-test");
        fs::create_dir_all(&root).unwrap();
        let accountless_namespace =
            format!("draft-{}", bytes_sha256(root.to_string_lossy().as_bytes()));
        let accountless_directory = state_directory(&accountless_namespace).unwrap();

        let (first_lock, first) =
            DraftState::acquire(&root, "account-a", LegacyStatePolicy::Reject)
                .await
                .unwrap();
        let first_directory = first.path.parent().unwrap().to_path_buf();
        drop(first_lock);
        let (second_lock, second) =
            DraftState::acquire(&root, "account-b", LegacyStatePolicy::Reject)
                .await
                .unwrap();
        let second_directory = second.path.parent().unwrap().to_path_buf();

        assert_ne!(first.path, second.path);
        drop(second_lock);
        let _ = fs::remove_dir_all(first_directory);
        let _ = fs::remove_dir_all(second_directory);
        let _ = fs::remove_dir_all(accountless_directory);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn legacy_state_requires_an_explicit_policy() {
        let root = temporary_root("legacy-reject-test");
        let legacy_directory = root.join("legacy");
        let legacy_path = legacy_directory.join("state.json");
        write_state(&legacy_path, "legacy");
        let mut current = empty_state();
        current.path = root.join("current/state.json");

        let error = current
            .handle_legacy(&root, &legacy_directory, LegacyStatePolicy::Reject)
            .await
            .unwrap_err();
        assert!(matches!(error, CliError::LegacyStatePolicyRequired));
        assert!(legacy_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn legacy_state_can_be_adopted_or_discarded_explicitly() {
        let adopt_root = temporary_root("legacy-adopt-test");
        let adopt_directory = adopt_root.join("legacy");
        let adopt_path = adopt_directory.join("state.json");
        write_state(&adopt_path, "adopted");
        let mut adopted = empty_state();
        adopted.path = adopt_root.join("current/state.json");
        adopted
            .handle_legacy(&adopt_root, &adopt_directory, LegacyStatePolicy::Adopt)
            .await
            .unwrap();
        assert!(adopted.has_pending());
        assert!(adopted.path.exists());
        assert!(!adopt_path.exists());
        let _ = fs::remove_dir_all(adopt_root);

        let discard_root = temporary_root("legacy-discard-test");
        let discard_directory = discard_root.join("legacy");
        let discard_path = discard_directory.join("state.json");
        write_state(&discard_path, "discarded");
        let mut discarded = empty_state();
        discarded.path = discard_root.join("current/state.json");
        discarded
            .handle_legacy(
                &discard_root,
                &discard_directory,
                LegacyStatePolicy::Discard,
            )
            .await
            .unwrap();
        assert!(!discarded.has_pending());
        assert!(!discard_path.exists());
        let _ = fs::remove_dir_all(discard_root);
    }

    #[tokio::test]
    async fn restores_a_backup_when_the_primary_state_is_missing() {
        let directory = temporary_root("state-backup-test");
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("state.json");
        let backup = target.with_extension("json.backup");
        let state = DraftState {
            version: STATE_VERSION,
            assets: BTreeMap::new(),
            pending_push: Some("pending".to_owned()),
            completed_pushes: BTreeMap::new(),
            path: target.clone(),
        };
        fs::write(&backup, serde_json::to_vec(&state).unwrap()).unwrap();

        let loaded = DraftState::load_path(target.clone()).await.unwrap();
        assert!(loaded.has_pending());
        assert!(target.exists());
        assert!(!backup.exists());
        let _ = fs::remove_dir_all(directory);
    }
}
