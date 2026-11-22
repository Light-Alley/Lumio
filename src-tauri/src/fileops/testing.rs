//! A stand-in for the app, a throwaway folder and a catalogue, for tests that work on
//! real files.
use super::*;

pub struct TestHost {
    pub conn: Mutex<Connection>,
    pub jobs: Mutex<Vec<Job>>,
}

impl Host for TestHost {
    fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }
    fn queue(&self, job: Job) {
        self.jobs.lock().push(job);
    }
    fn forget(&self, conn: &Connection, ids: &[i64]) -> Result<()> {
        for id in ids {
            conn.execute("DELETE FROM photos WHERE id = ?1", [id])?;
        }
        Ok(())
    }
}

/// A temporary folder, removed when dropped.
pub struct Tmp(pub PathBuf);
impl Tmp {
    pub fn new(tag: &str) -> Tmp {
        // A fresh folder with an unguessable name (removed by Drop below).
        Tmp(tempfile::Builder::new().prefix(&format!("lumio-{tag}-")).tempdir().unwrap().keep())
    }
    /// `t.join("a/b.jpg")` with the platform's separator, so paths compare equal to
    /// the ones Lumio builds (on Windows "a\\b.jpg", not "a/b.jpg").
    pub fn join(&self, s: &str) -> PathBuf {
        s.split('/').filter(|c| !c.is_empty()).fold(self.0.clone(), |p, c| p.join(c))
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn host() -> TestHost {
    let conn = Connection::open_in_memory().unwrap();
    db::migrate(&conn).unwrap();
    TestHost { conn: Mutex::new(conn), jobs: Mutex::new(vec![]) }
}

impl TestHost {
    pub fn root(&self, path: &Path) -> i64 {
        let c = self.conn.lock();
        c.execute("INSERT INTO roots (path, name, added_at) VALUES (?1, 'r', 0)", [path.to_string_lossy()]).unwrap();
        c.last_insert_rowid()
    }
    /// Write a file (its content is its own name unless given) and catalogue it unless it's a sidecar.
    pub fn file(&self, root: i64, path: &Path, content: Option<&str>) -> i64 {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        std::fs::write(path, content.unwrap_or(&name)).unwrap();
        if indexer::ext_of(path) == "xmp" {
            return 0;
        }
        self.photo(root, path)
    }
    /// Catalogue a photo without making the file.
    pub fn photo(&self, root: i64, path: &Path) -> i64 {
        let c = self.conn.lock();
        let size = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(5);
        c.execute(
            "INSERT INTO photos (root_id, path, folder, filename, ext, size, rating) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 4)",
            params![
                root,
                path.to_string_lossy(),
                path.parent().unwrap().to_string_lossy(),
                path.file_name().unwrap().to_string_lossy(),
                indexer::ext_of(path),
                size
            ],
        )
        .unwrap();
        c.last_insert_rowid()
    }
    pub fn path_of(&self, id: i64) -> Option<String> {
        db::get_photo(&self.conn.lock(), id).unwrap().map(|p| p.path)
    }
    pub fn rating(&self, id: i64) -> i64 {
        db::get_photo(&self.conn.lock(), id).unwrap().unwrap().rating
    }
}

pub fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = match std::fs::read_dir(dir) {
        // Finder leaves a .DS_Store behind when it puts files in the Trash.
        Ok(rd) => {
            rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n != ".DS_Store").collect()
        }
        Err(_) => vec![],
    };
    v.sort();
    v
}

pub fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}
