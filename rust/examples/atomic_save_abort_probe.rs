//! Windows-only process-abort boundary probe. This never scans user folders.
//! Run the release example directly; Cargo test does not use panic=abort.
//! Requires a trusted, stable temp parent and ancestors without reparse changes,
//! matching the production writer's namespace boundary. Directory creation and
//! opening are not one atomic operation: even empty fixture roots are retained.
//! Only create_new-owned or writer-handle-identified files are removed by handle.
#[cfg(windows)]
mod probe {
    use shotkit::atomic_save_windows::{write_atomic_with, write_new_unique_with};
    use std::{
        cell::Cell,
        ffi::c_void,
        fs::{self, File, OpenOptions},
        io::{self, BufRead, Write},
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::mpsc,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    const READ: u32 = 0x8000_0000;
    const WRITE: u32 = 0x4000_0000;
    const DELETE: u32 = 0x0001_0000;
    const SHARE_READ: u32 = 1;
    const SHARE_ALL: u32 = 7;
    const PAYLOAD: &[u8] = b"owned partial or prepared bytes";
    const OLD: &[u8] = b"existing target";
    const FOREIGN: &[u8] = b"foreign to the writer; owned by this fixture";
    const DEADLINE: Duration = Duration::from_secs(15);

    #[repr(C)]
    #[derive(Default)]
    struct Information {
        attributes: u32,
        times: [u32; 6],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(handle: *mut c_void, info: *mut Information) -> i32;
        fn SetFileInformationByHandle(
            handle: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
        fn SetErrorMode(mode: u32) -> u32;
    }

    type Identity = (u32, u32, u32);

    fn identity(file: &File) -> io::Result<Identity> {
        let mut info = Information::default();
        // SAFETY: live owned handle and correctly laid out writable structure.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((info.volume, info.index_high, info.index_low))
    }

    fn check(condition: bool, message: &str) -> io::Result<()> {
        if condition {
            Ok(())
        } else {
            Err(io::Error::other(message))
        }
    }

    fn open(path: &Path, share: u32, delete: bool) -> io::Result<File> {
        OpenOptions::new()
            .access_mode(READ | if delete { DELETE } else { 0 })
            .share_mode(share)
            .open(path)
    }

    fn bytes(file: &File) -> io::Result<Vec<u8>> {
        use std::os::windows::fs::FileExt;
        let length = usize::try_from(file.metadata()?.len()).map_err(io::Error::other)?;
        check(length < 4096, "unexpected fixture file size")?;
        let mut data = vec![0; length];
        let count = file.seek_read(&mut data, 0)?;
        check(count == length, "short fixture read")?;
        Ok(data)
    }

    fn delete_owned(file: File, expected: Identity) -> io::Result<()> {
        check(identity(&file)? == expected, "cleanup identity mismatch")?;
        let disposition = 1u8;
        // SAFETY: DELETE-capable live handle; FILE_DISPOSITION_INFO is one BOOLEAN.
        // Cleanup never resolves a path again after verifying the held object.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                4,
                (&disposition as *const u8).cast(),
                1,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        drop(file);
        Ok(())
    }

    struct Owned {
        path: PathBuf,
        id: Identity,
    }

    struct Fixture {
        root: PathBuf,
        directory: File,
        directory_id: Identity,
        known: Vec<Owned>,
        // These guards deny replacement throughout the abort/error checks.
        guards: Vec<File>,
    }

    impl Fixture {
        fn new(existing_target: bool) -> io::Result<Self> {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos();
            let mut root = None;
            for sequence in 0..128 {
                let candidate = std::env::temp_dir().join(format!(
                    "shotkit-abort-probe-{}-{stamp}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&candidate) {
                    Ok(()) => {
                        root = Some(candidate);
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
            }
            let root = root.ok_or_else(|| io::Error::other("fixture name exhausted"))?;
            // Deny delete sharing after open. This does not prove that the opened
            // directory is the one created above, so never delete the root itself.
            let directory = OpenOptions::new()
                .access_mode(READ)
                .share_mode(3)
                .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS
                .open(&root)?;
            let directory_id = identity(&directory)?;
            let mut fixture = Self {
                root,
                directory,
                directory_id,
                known: Vec::new(),
                guards: Vec::new(),
            };
            fixture.create(".shotkit-foreign-held.tmp", FOREIGN, true)?;
            fixture.create(".shotkit-foreign-open.tmp", FOREIGN, false)?;
            if existing_target {
                fixture.create("target.png", OLD, true)?;
            }
            Ok(fixture)
        }

        fn create(&mut self, name: &str, data: &[u8], held: bool) -> io::Result<()> {
            let path = self.root.join(name);
            let mut file = OpenOptions::new()
                .write(true)
                .access_mode(READ | WRITE | DELETE)
                .share_mode(SHARE_READ)
                .create_new(true)
                .open(&path)?;
            file.write_all(data)?;
            file.sync_all()?;
            let id = identity(&file)?;
            self.known.push(Owned { path, id });
            if held {
                // Keep the original create_new handle, without a close/reopen gap.
                self.guards.push(file);
            }
            Ok(())
        }

        fn verify_known(&self) -> io::Result<()> {
            check(
                identity(&self.directory)? == self.directory_id,
                "directory changed",
            )?;
            for owned in &self.known {
                let file = open(&owned.path, SHARE_ALL, false)?;
                check(identity(&file)? == owned.id, "fixture identity changed")?;
                let expected = if owned.path.file_name().unwrap() == "target.png" {
                    OLD
                } else {
                    FOREIGN
                };
                check(bytes(&file)? == expected, "fixture bytes changed")?;
            }
            Ok(())
        }

        fn unknown(&self) -> io::Result<Vec<PathBuf>> {
            fs::read_dir(&self.root)?
                .filter_map(|entry| match entry {
                    Ok(entry) if self.known.iter().any(|item| item.path == entry.path()) => None,
                    Ok(entry) => Some(Ok(entry.path())),
                    Err(error) => Some(Err(error)),
                })
                .collect()
        }

        fn cleanup(mut self) -> io::Result<()> {
            self.verify_known()?;
            check(
                self.unknown()?.is_empty(),
                "unregistered entry; retain fixture",
            )?;
            self.guards.clear();
            // Hold every expected object before deleting any of them.
            let files: Vec<_> = self
                .known
                .iter()
                .map(|owned| {
                    let file = open(&owned.path, SHARE_READ, true)?;
                    check(identity(&file)? == owned.id, "cleanup identity changed")?;
                    Ok((file, owned.id))
                })
                .collect::<io::Result<_>>()?;
            for (file, id) in files {
                delete_owned(file, id)?;
            }
            check(
                fs::read_dir(&self.root)?.next().is_none(),
                "directory not empty",
            )?;
            // Runner temp storage expires externally. Do not delete this directory:
            // create_dir -> open did not atomically establish directory ownership.
            Ok(())
        }
    }

    struct Running(Child);

    impl Drop for Running {
        fn drop(&mut self) {
            match self.0.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {}
                Err(error) => eprintln!("Child status check failed during teardown: {error}"),
            }
            if let Err(error) = self.0.kill() {
                eprintln!("Child termination request failed: {error}");
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                match self.0.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    Err(error) => {
                        eprintln!("Child teardown status check failed: {error}");
                        return;
                    }
                }
            }
            // Only our waiting time is bounded. If kill/status checks fail,
            // actual child termination is not guaranteed. The parent may exit 2
            // before the CI step timeout; that timeout is no termination proof.
            eprintln!("Child teardown deadline exceeded; child termination is unconfirmed");
        }
    }

    fn checkpoint(id: Identity) -> io::Result<()> {
        println!("READY {} {} {}", id.0, id.1, id.2);
        io::stdout().flush()?;
        let mut command = String::new();
        io::stdin().read_line(&mut command)?;
        check(command == "GO\n", "missing parent handshake")?;
        println!("PANIC_ABORT_ARMED");
        io::stdout().flush()?;
        panic!("SHOTKIT_ABORT_PROBE: intentional release panic");
    }

    fn child(root: &Path, mode: &str) -> io::Result<()> {
        check(cfg!(panic = "abort"), "helper must use panic=abort")?;
        check(
            matches!(
                mode,
                "atomic-old" | "atomic-new" | "unique-prepare" | "unique-publish"
            ),
            "invalid child mode",
        )?;
        // SAFETY: process-local error-mode setting prevents an unattended crash dialog.
        unsafe { SetErrorMode(0x0002 | 0x8000) };
        let target = root.join("target.png");
        if mode == "unique-publish" {
            let prepared_id = Cell::new(None);
            let _ = write_new_unique_with(
                root,
                1,
                || {
                    checkpoint(prepared_id.get().expect("prepare must run first")).unwrap();
                    target.clone()
                },
                |file| {
                    file.write_all(PAYLOAD)?;
                    prepared_id.set(Some(identity(file)?));
                    Ok(())
                },
            );
        } else {
            let prepare = |file: &mut File| {
                file.write_all(PAYLOAD)?;
                file.sync_all()?;
                checkpoint(identity(file)?)
            };
            if mode == "unique-prepare" {
                let _ = write_new_unique_with(root, 1, || target.clone(), prepare);
            } else {
                let _ = write_atomic_with(&target, mode == "atomic-old", prepare);
            }
        }
        Err(io::Error::other("child returned instead of aborting"))
    }

    fn abort_case(mode: &str) -> io::Result<()> {
        let fixture = Fixture::new(mode != "atomic-new")?;
        let child = Command::new(std::env::current_exe()?)
            .arg("--child")
            .arg(&fixture.root)
            .arg(mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut running = Running(child);
        let stdout = running.0.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let _reader = std::thread::spawn(move || {
            for line in io::BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let ready = receiver
            .recv_timeout(DEADLINE)
            .map_err(io::Error::other)??;
        let values: Vec<_> = ready.split_whitespace().collect();
        check(values.len() == 4 && values[0] == "READY", "missing READY")?;
        let parse = |value: &str| value.parse::<u32>().map_err(io::Error::other);
        let id = (parse(values[1])?, parse(values[2])?, parse(values[3])?);
        let new = fixture.unknown()?;
        check(new.len() == 1, "expected one owned temporary file")?;
        let temp = &new[0];
        check(
            temp.extension().is_some_and(|ext| ext == "tmp"),
            "unexpected temp name",
        )?;
        {
            // Share all is required while the writer holds WRITE and DELETE access.
            let file = open(temp, SHARE_ALL, false)?;
            check(identity(&file)? == id, "READY temp identity mismatch")?;
            check(bytes(&file)? == PAYLOAD, "READY payload mismatch")?;
            check(fs::remove_file(temp).is_err(), "live temp was deletable")?;
            check(
                fs::rename(temp, fixture.root.join("stolen")).is_err(),
                "live temp moved",
            )?;
            check(
                fs::write(temp, b"foreign").is_err(),
                "live temp was writable",
            )?;
        } // Do not keep a temp handle alive across process abort.
        fixture.verify_known()?;
        let mut stdin = running.0.stdin.take().unwrap();
        stdin.write_all(b"GO\n")?;
        drop(stdin);
        let armed = receiver
            .recv_timeout(DEADLINE)
            .map_err(io::Error::other)??;
        check(armed == "PANIC_ABORT_ARMED", "abort was not armed")?;
        let deadline = Instant::now() + DEADLINE;
        let status = loop {
            if let Some(status) = running.0.try_wait()? {
                break status;
            }
            check(Instant::now() < deadline, "abort deadline exceeded")?;
            std::thread::sleep(Duration::from_millis(10));
        };
        check(
            !status.success() && status.code() != Some(2),
            "expected abnormal abort exit",
        )?;
        fixture.verify_known()?;
        check(fixture.unknown()? == new, "post-abort namespace changed")?;
        let file = open(temp, SHARE_READ, false)?;
        check(identity(&file)? == id, "post-abort temp identity changed")?;
        check(bytes(&file)? == PAYLOAD, "post-abort payload changed")?;
        drop(file);
        if mode == "atomic-new" {
            check(
                !fixture.root.join("target.png").exists(),
                "incomplete target published",
            )?;
        }
        // Existing files were checked above; remove only the verified owned remnant.
        cleanup_output(fixture, temp, id)?;
        println!("ABORT_BOUNDARY_OK {mode}: owned temp retained; target and decoys preserved");
        Ok(())
    }

    fn cleanup_output(mut fixture: Fixture, output: &Path, id: Identity) -> io::Result<()> {
        let file = open(output, SHARE_READ, true)?;
        delete_owned(file, id)?;
        fixture.known.retain(|owned| owned.path != output);
        fixture.cleanup()
    }

    fn controls(unique: bool, success: bool) -> io::Result<()> {
        let fixture = Fixture::new(!success)?;
        let target = fixture.root.join("target.png");
        let preparations = Cell::new(0);
        let prepared_identity = Cell::new(None);
        let prepare = |file: &mut File| {
            preparations.set(preparations.get() + 1);
            prepared_identity.set(Some(identity(file)?));
            file.write_all(PAYLOAD)?;
            if success {
                Ok(())
            } else {
                Err(io::Error::other("controlled preparation error"))
            }
        };
        let result = if unique {
            write_new_unique_with(&fixture.root, 1, || target.clone(), prepare)
                .map(|_| ())
                .map_err(|error| error.to_string())
        } else {
            write_atomic_with(&target, !success, prepare)
        };
        check(
            result.is_ok() == success,
            "control returned unexpected result",
        )?;
        check(preparations.get() == 1, "preparation must run exactly once")?;
        if !success {
            check(
                result.unwrap_err().contains("controlled preparation error"),
                "control lost injected preparation error",
            )?;
        }
        fixture.verify_known()?;
        if success {
            check(
                fixture.unknown()? == vec![target.clone()],
                "success namespace mismatch",
            )?;
            let file = open(&target, SHARE_READ, false)?;
            let id = identity(&file)?;
            check(
                Some(id) == prepared_identity.get(),
                "published identity mismatch",
            )?;
            check(bytes(&file)? == PAYLOAD, "published payload mismatch")?;
            drop(file);
            cleanup_output(fixture, &target, id)?;
        } else {
            fixture.cleanup()?;
        }
        println!("CONTROL_OK unique={unique} success={success}");
        Ok(())
    }

    fn existing_target_success() -> io::Result<()> {
        let mut fixture = Fixture::new(true)?;
        let target = fixture.root.join("target.png");
        fixture.verify_known()?;
        let old_id = fixture
            .known
            .iter()
            .find(|owned| owned.path == target)
            .ok_or_else(|| io::Error::other("missing owned target"))?
            .id;
        let guard_index = fixture
            .guards
            .iter()
            .position(|file| identity(file).is_ok_and(|id| id == old_id))
            .ok_or_else(|| io::Error::other("missing target guard"))?;
        // Replacement must release the guard that intentionally forbids delete.
        // This is only the fixture's create_new-owned target, in its trusted
        // private namespace. Production concurrent-target replacement is not tested.
        drop(fixture.guards.remove(guard_index));
        let prepared_id = Cell::new(None);
        let preparations = Cell::new(0);
        write_atomic_with(&target, true, |file| {
            preparations.set(preparations.get() + 1);
            prepared_id.set(Some(identity(file)?));
            file.write_all(PAYLOAD)
        })
        .map_err(io::Error::other)?;
        check(
            preparations.get() == 1,
            "replacement must prepare exactly once",
        )?;
        // The old owned target was replaced deliberately; retain all decoy checks.
        fixture.known.retain(|owned| owned.path != target);
        fixture.verify_known()?;
        check(
            fixture.unknown()? == vec![target.clone()],
            "replacement namespace mismatch",
        )?;
        let file = open(&target, SHARE_READ, false)?;
        let id = identity(&file)?;
        check(
            Some(id) == prepared_id.get(),
            "replacement identity mismatch",
        )?;
        check(id != old_id, "replacement retained old object")?;
        check(bytes(&file)? == PAYLOAD, "replacement payload mismatch")?;
        drop(file);
        cleanup_output(fixture, &target, id)?;
        println!("CONTROL_OK atomic existing-target replacement");
        Ok(())
    }

    pub fn run() -> io::Result<()> {
        check(cfg!(panic = "abort"), "run this example with --release")?;
        let args: Vec<_> = std::env::args_os().collect();
        if args.get(1).is_some_and(|arg| arg == "--child") {
            check(args.len() == 4, "invalid child arguments")?;
            return child(
                Path::new(&args[2]),
                args[3]
                    .to_str()
                    .ok_or_else(|| io::Error::other("invalid mode"))?,
            );
        }
        check(args.len() == 1, "unexpected arguments")?;
        for mode in [
            "atomic-old",
            "atomic-new",
            "unique-prepare",
            "unique-publish",
        ] {
            abort_case(mode)?;
        }
        for unique in [false, true] {
            for success in [false, true] {
                controls(unique, success)?;
            }
        }
        existing_target_success()?;
        println!("ATOMIC_SAVE_ABORT_BOUNDARY_OK: retention is expected, not crash cleanup");
        Ok(())
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = probe::run() {
        eprintln!("Atomic-save abort probe failed; fixture retained: {error}");
        std::process::exit(2);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This probe requires Windows and --release.");
    std::process::exit(2);
}
