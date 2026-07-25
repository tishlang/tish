//! Zip extraction for Tish (`tish:zip`), behind the `zip` feature.
//!
//!   import { extract, entries } from 'tish:zip'
//!
//!   - `extract(zipPath, destDir) -> number`   entries written under destDir; -1 on open error.
//!                                             Entries with an unsafe path (absolute, or containing
//!                                             `..`) are SKIPPED, not written. Unix mode preserved.
//!   - `entries(zipPath) -> string[] | null`   the archive's entry names (null on open error).
//!
//! Generic archive extraction — a VSIX is just a zip, so this backs openvsx install; any archive-
//! specific prefix handling (VSIX entries live under `extension/`) is the caller's job.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use tishlang_core::{Value, VmRef};

fn arg_str(args: &[Value], i: usize) -> Option<String> {
    match args.get(i) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// Join an archive entry name under `dest`, rejecting any escape: an absolute path, a Windows
/// prefix/root, or a `..` component. Returns None for an unsafe entry (the caller skips it).
fn safe_join(dest: &Path, name: &str) -> Option<PathBuf> {
    let mut out = dest.to_path_buf();
    for comp in Path::new(name).components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            _ => return None, // ParentDir / RootDir / Prefix → escape
        }
    }
    // A name of only "." / "" yields `dest` itself — nothing to write.
    if out == dest {
        return None;
    }
    Some(out)
}

pub fn zip_extract(args: &[Value]) -> Value {
    let zip_path = match arg_str(args, 0) {
        Some(p) => p,
        None => return Value::Number(-1.0),
    };
    let dest = match arg_str(args, 1) {
        Some(p) => p,
        None => return Value::Number(-1.0),
    };
    let file = match fs::File::open(&zip_path) {
        Ok(f) => f,
        Err(_) => return Value::Number(-1.0),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return Value::Number(-1.0),
    };
    let dest_path = Path::new(&dest);
    let mut count: i64 = 0;
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let out_path = match safe_join(dest_path, entry.name()) {
            Some(p) => p,
            None => continue, // skip unsafe / empty entries
        };
        if entry.is_dir() {
            let _ = fs::create_dir_all(&out_path);
            continue;
        }
        if let Some(parent) = out_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_err() {
            continue;
        }
        if fs::write(&out_path, &buf).is_err() {
            continue;
        }
        // Preserve the stored unix mode (executable bit etc.) — a bundled native binary extracted
        // as rw-r--r-- would fail to launch.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode));
            }
        }
        count += 1;
    }
    Value::Number(count as f64)
}

pub fn zip_entries(args: &[Value]) -> Value {
    let zip_path = match arg_str(args, 0) {
        Some(p) => p,
        None => return Value::Null,
    };
    let file = match fs::File::open(&zip_path) {
        Ok(f) => f,
        Err(_) => return Value::Null,
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return Value::Null,
    };
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        if let Ok(e) = archive.by_index(i) {
            out.push(Value::String(e.name().to_string().into()));
        }
    }
    Value::Array(VmRef::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(path: &Path) {
        let f = fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        w.start_file("a.txt", opts).unwrap();
        w.write_all(b"hello").unwrap();
        w.start_file("sub/b.txt", opts).unwrap();
        w.write_all(b"nested").unwrap();
        // A traversal entry that MUST be skipped (never written outside dest).
        w.start_file("../evil.txt", opts).unwrap();
        w.write_all(b"pwned").unwrap();
        w.finish().unwrap();
    }

    #[test]
    fn extract_writes_safe_entries_and_skips_traversal() {
        let dir = std::env::temp_dir().join(format!("tishzip_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("t.zip");
        make_zip(&zip_path);
        let dest = dir.join("out");

        let n = zip_extract(&[
            Value::String(zip_path.to_string_lossy().to_string().into()),
            Value::String(dest.to_string_lossy().to_string().into()),
        ]);
        // 2 safe files written; the ../evil.txt entry skipped.
        assert!(matches!(n, Value::Number(x) if x == 2.0), "got {n:?}");
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(dest.join("sub/b.txt")).unwrap(), "nested");
        // The traversal entry must NOT have escaped to the parent of dest.
        assert!(!dir.join("evil.txt").exists(), "traversal entry escaped!");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entries_lists_names() {
        let dir = std::env::temp_dir().join(format!("tishzipe_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("t.zip");
        make_zip(&zip_path);
        let v = zip_entries(&[Value::String(zip_path.to_string_lossy().to_string().into())]);
        let Value::Array(a) = v else { panic!("not array") };
        let names: Vec<String> = a
            .borrow()
            .iter()
            .map(|x| x.to_display_string())
            .collect();
        assert!(names.contains(&"a.txt".to_string()));
        assert!(names.contains(&"sub/b.txt".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }
}
