use super::testing::*;
use super::*;

#[cfg(unix)]
#[test]
fn links_out_of_a_root_are_refused() {
    let lib = Tmp::new("lib");
    let outside = Tmp::new("outside");
    std::fs::write(outside.join("secret.txt"), "x").unwrap();
    std::fs::create_dir_all(lib.join("Trip")).unwrap();
    std::os::unix::fs::symlink(&outside.0, lib.join("escape_link")).unwrap();
    assert!(inside_root(&lib.0, &lib.join("Trip")).is_ok());
    assert!(inside_root(&lib.0, &lib.join("Trip/New folder")).is_ok(), "a folder about to be created");
    assert!(inside_root(&lib.0, &lib.join("escape_link")).is_err());
    assert!(inside_root(&lib.0, &lib.join("escape_link/sub/new")).is_err());
    assert!(not_a_link(&lib.join("escape_link")).is_err());
    assert!(not_a_link(&lib.join("Trip")).is_ok());
    // Unplugged drive: nothing to follow.
    assert!(inside_root(Path::new("/nonexistent-frameloft-root"), Path::new("/nonexistent-frameloft-root/a")).is_ok());
}

#[cfg(unix)]
#[test]
fn copying_a_folder_skips_links_and_special_files() {
    let t = Tmp::new("copylinks");
    let outside = Tmp::new("copyout");
    std::fs::write(outside.join("private.txt"), "secret").unwrap();
    std::fs::create_dir_all(t.join("src/sub")).unwrap();
    std::fs::write(t.join("src/a.jpg"), "a").unwrap();
    std::fs::write(t.join("src/sub/b.jpg"), "b").unwrap();
    std::os::unix::fs::symlink(outside.join("private.txt"), t.join("src/link.txt")).unwrap();
    std::os::unix::fs::symlink(&outside.0, t.join("src/dirlink")).unwrap();
    let fifo = t.join("src/pipe");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    copy_dir_recursive(&t.join("src"), &t.join("dst")).unwrap();
    assert_eq!(names(&t.join("dst")), vec!["a.jpg", "sub"]);
    assert_eq!(names(&t.join("dst/sub")), vec!["b.jpg"]);
}

#[test]
fn same_place_follows_the_drive() {
    let t = Tmp::new("case");
    std::fs::write(t.join("IMG_1.jpg"), "a").unwrap();
    assert!(same_place(&t.join("IMG_1.jpg"), &t.join("IMG_1.jpg")));
    assert!(!same_place(&t.join("IMG_1.jpg"), &t.join("IMG_2.jpg")));
    let upper = t.join("IMG_1.JPG");
    if upper.exists() {
        // This drive ignores case: both names are the one file.
        assert!(same_place(&t.join("IMG_1.jpg"), &upper));
    } else {
        std::fs::write(&upper, "b").unwrap();
        // Case-sensitive drive: two different files.
        assert!(!same_place(&t.join("IMG_1.jpg"), &upper));
    }
}

#[cfg(unix)]
#[test]
fn a_failed_folder_move_never_deletes_the_original() {
    use std::os::unix::fs::PermissionsExt;
    let h = host();
    let t = Tmp::new("mvfail");
    let r = h.root(&t.0);
    h.file(r, &t.join("A/x.jpg"), None);
    std::fs::create_dir_all(t.join("B")).unwrap();
    // A destination we may not write into: rename fails (not a cross-device error).
    std::fs::set_permissions(t.join("B"), std::fs::Permissions::from_mode(0o555)).unwrap();
    let res = move_folder(&h, &t.join("A"), &t.join("B"), false);
    std::fs::set_permissions(t.join("B"), std::fs::Permissions::from_mode(0o755)).unwrap();
    if nix_is_root() {
        return; // root ignores permissions, so the move just works
    }
    assert!(res.is_err());
    assert!(t.join("A/x.jpg").exists());
}

#[cfg(unix)]
fn nix_is_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

#[test]
fn move_file_never_replaces_an_existing_file() {
    let t = Tmp::new("fo");
    let (a, b) = (t.join("a.jpg"), t.join("b.jpg"));
    std::fs::write(&a, "A").unwrap();
    std::fs::write(&b, "B").unwrap();
    assert!(move_file(&a, &b).is_err());
    assert_eq!((read(&a), read(&b)), ("A".into(), "B".into()));
    assert!(copy_verified(&a, &b).is_err());
    assert_eq!(read(&b), "B", "a refused copy must not touch the file that was there");
    // Into a free name it works, and leaves no temporary files behind.
    move_file(&a, &t.join("c.jpg")).unwrap();
    assert_eq!(names(&t.0), ["b.jpg", "c.jpg"]);
}

#[test]
fn copies_are_checked_and_keep_the_date() {
    let t = Tmp::new("fo");
    let a = t.join("a.jpg");
    std::fs::write(&a, vec![7u8; 3_000_000]).unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
    std::fs::File::options().write(true).open(&a).unwrap().set_modified(old).unwrap();
    let c = t.join("copy.jpg");
    copy_verified(&a, &c).unwrap();
    assert!(same_content(&a, &c).unwrap());
    assert_eq!(std::fs::metadata(&c).unwrap().modified().unwrap(), old);
    assert_eq!(names(&t.0), ["a.jpg", "copy.jpg"]);
    std::fs::write(&c, vec![7u8; 2_999_999]).unwrap();
    assert!(!same_content(&a, &c).unwrap());
    let mut d = vec![7u8; 3_000_000];
    d[2_500_000] = 8;
    std::fs::write(&c, d).unwrap();
    assert!(!same_content(&a, &c).unwrap());
}

#[test]
fn copying_from_a_missing_file_leaves_nothing_behind() {
    let t = Tmp::new("fo");
    assert!(copy_verified(&t.join("gone.jpg"), &t.join("new.jpg")).is_err());
    assert!(names(&t.0).is_empty());
}

#[test]
fn moves_photo_sidecar_and_catalogue() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let id = h.file(r, &t.join("in/IMG_1.CR2"), None);
    h.file(r, &t.join("in/IMG_1.xmp"), None);
    std::fs::create_dir_all(t.join("out")).unwrap();
    let rep = move_photos(&h, &[id], &t.join("out"), false).unwrap();
    assert_eq!((rep.done, rep.failed.len()), (1, 0));
    assert_eq!(names(&t.join("out")), ["IMG_1.CR2", "IMG_1.xmp"]);
    assert!(names(&t.join("in")).is_empty());
    assert_eq!(h.path_of(id).unwrap(), t.join("out/IMG_1.CR2").to_string_lossy());
    assert_eq!(h.rating(id), 4, "the catalogue entry (and its rating) moves with the file");
    assert_eq!(rep.moved.len(), 1);
    assert_eq!(rep.moved[0].from, t.join("in/IMG_1.CR2").to_string_lossy());
}

#[test]
fn moving_never_overwrites_a_photo_or_a_sidecar() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let id = h.file(r, &t.join("in/IMG_1.jpg"), Some("mine"));
    h.file(r, &t.join("in/IMG_1.xmp"), Some("my sidecar"));
    // The destination has a different IMG_1.jpg, and an IMG_1.xmp that belongs to an IMG_1.CR2.
    h.file(r, &t.join("out/IMG_1.jpg"), Some("theirs"));
    h.file(r, &t.join("out/IMG_1.CR2"), Some("raw"));
    h.file(r, &t.join("out/IMG_1.xmp"), Some("their sidecar"));
    let rep = move_photos(&h, &[id], &t.join("out"), false).unwrap();
    assert_eq!(rep.done, 1, "{:?}", rep.failed);
    let out = t.join("out");
    assert_eq!(read(&out.join("IMG_1.jpg")), "theirs");
    assert_eq!(read(&out.join("IMG_1.xmp")), "their sidecar");
    assert_eq!(read(&out.join("IMG_1 (2).jpg")), "mine");
    assert_eq!(read(&out.join("IMG_1 (2).xmp")), "my sidecar");
}

#[test]
fn sidecar_name_taken_even_when_photo_name_is_free() {
    // Only the sidecar's name is taken: the photo must not take a name whose sidecar
    // would land on someone else's.
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let id = h.file(r, &t.join("in/IMG_1.jpg"), Some("mine"));
    h.file(r, &t.join("in/IMG_1.xmp"), Some("my sidecar"));
    h.file(r, &t.join("out/IMG_1.CR2"), Some("raw"));
    h.file(r, &t.join("out/IMG_1.xmp"), Some("their sidecar"));
    let rep = move_photos(&h, &[id], &t.join("out"), false).unwrap();
    assert_eq!(rep.done, 1);
    assert_eq!(read(&t.join("out/IMG_1.xmp")), "their sidecar");
    assert_eq!(read(&t.join("out/IMG_1 (2).xmp")), "my sidecar");
    assert_eq!(read(&t.join("out/IMG_1 (2).jpg")), "mine");
}

#[test]
fn moving_the_raw_of_a_raw_jpeg_pair_takes_its_sidecar() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let raw = h.file(r, &t.join("in/IMG_1.CR2"), None);
    h.file(r, &t.join("in/IMG_1.JPG"), None);
    h.file(r, &t.join("in/IMG_1.xmp"), Some("edits"));
    std::fs::create_dir_all(t.join("out")).unwrap();
    move_photos(&h, &[raw], &t.join("out"), false).unwrap();
    // No copy is left behind: the JPEG would otherwise start showing the RAW's rating.
    assert_eq!(names(&t.join("in")), ["IMG_1.JPG"]);
    assert_eq!(names(&t.join("out")), ["IMG_1.CR2", "IMG_1.xmp"]);
}

#[test]
fn moving_the_jpeg_of_a_raw_jpeg_pair_leaves_the_raw_sidecar() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let raw = h.file(r, &t.join("in/IMG_1.CR2"), None);
    let jpg = h.file(r, &t.join("in/IMG_1.JPG"), None);
    h.file(r, &t.join("in/IMG_1.xmp"), Some("edits"));
    std::fs::create_dir_all(t.join("out")).unwrap();
    move_photos(&h, &[jpg], &t.join("out"), false).unwrap();
    assert_eq!(names(&t.join("in")), ["IMG_1.CR2", "IMG_1.xmp"], "the RAW keeps its sidecar");
    // "IMG_1.xmp" is the RAW's (Lightroom's naming), so the JPEG leaves without it.
    assert_eq!(names(&t.join("out")), ["IMG_1.JPG"]);
    // Moving the RAW after it brings the sidecar along.
    move_photos(&h, &[raw], &t.join("out"), false).unwrap();
    assert!(names(&t.join("in")).is_empty());
    assert_eq!(names(&t.join("out")), ["IMG_1.CR2", "IMG_1.JPG", "IMG_1.xmp"]);
    assert_eq!(read(&t.join("out/IMG_1.xmp")), "edits");
}

#[test]
fn moving_the_whole_pair_moves_the_sidecar() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let raw = h.file(r, &t.join("in/IMG_1.CR2"), None);
    let jpg = h.file(r, &t.join("in/IMG_1.JPG"), None);
    h.file(r, &t.join("in/IMG_1.xmp"), None);
    std::fs::create_dir_all(t.join("out")).unwrap();
    let rep = move_photos(&h, &[raw, jpg], &t.join("out"), false).unwrap();
    assert_eq!(rep.done, 2);
    assert!(names(&t.join("in")).is_empty());
    assert_eq!(names(&t.join("out")), ["IMG_1.CR2", "IMG_1.JPG", "IMG_1.xmp"]);
}

#[test]
fn two_photos_with_the_same_name_from_different_folders() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let a = h.file(r, &t.join("a/IMG_1.jpg"), Some("a"));
    let b = h.file(r, &t.join("b/IMG_1.jpg"), Some("b"));
    std::fs::create_dir_all(t.join("out")).unwrap();
    let rep = move_photos(&h, &[a, b], &t.join("out"), false).unwrap();
    assert_eq!(rep.done, 2);
    assert_eq!(names(&t.join("out")), ["IMG_1 (2).jpg", "IMG_1.jpg"]);
    assert_eq!(read(&t.join("out/IMG_1 (2).jpg")), "b");
    assert_eq!(h.path_of(b).unwrap(), t.join("out/IMG_1 (2).jpg").to_string_lossy());
}

#[test]
fn copy_keeps_the_original_and_queues_indexing() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let id = h.file(r, &t.join("in/IMG_1.jpg"), None);
    h.file(r, &t.join("in/IMG_1.xmp"), None);
    std::fs::create_dir_all(t.join("out")).unwrap();
    let rep = move_photos(&h, &[id], &t.join("out"), true).unwrap();
    assert_eq!(rep.done, 1);
    assert_eq!(names(&t.join("in")), ["IMG_1.jpg", "IMG_1.xmp"]);
    assert_eq!(names(&t.join("out")), ["IMG_1.jpg", "IMG_1.xmp"]);
    assert_eq!(rep.copies.len(), 2);
    assert!(h.jobs.lock().iter().any(|j| matches!(j, Job::Files(f) if f.len() == 1)));
    // Copying into the same folder makes a numbered copy.
    let rep = move_photos(&h, &[id], &t.join("in"), true).unwrap();
    assert_eq!(rep.done, 1);
    assert_eq!(names(&t.join("in")), ["IMG_1 (2).jpg", "IMG_1 (2).xmp", "IMG_1.jpg", "IMG_1.xmp"]);
}

#[test]
fn missing_file_on_a_connected_drive_fails_without_changes() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    std::fs::create_dir_all(t.join("in")).unwrap();
    std::fs::create_dir_all(t.join("out")).unwrap();
    let id = h.photo(r, &t.join("in/gone.jpg"));
    let rep = move_photos(&h, &[id], &t.join("out"), false).unwrap();
    assert_eq!((rep.done, rep.queued, rep.failed.len()), (0, 0, 1));
    assert_eq!(h.path_of(id).unwrap(), t.join("in/gone.jpg").to_string_lossy());
}

#[test]
fn unavailable_destination_is_refused() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.join("lib"));
    let id = h.file(r, &t.join("lib/a.jpg"), None);
    // Not a library folder, and not there.
    assert!(move_photos(&h, &[id], &t.join("nowhere"), false).is_err());
    assert!(t.join("lib/a.jpg").exists());
}

#[test]
fn moving_to_a_drive_that_is_away_waits_and_copying_is_refused() {
    let t = Tmp::new("fo");
    let h = host();
    let ra = h.root(&t.join("a"));
    h.root(&t.join("b")); // drive b isn't connected: its folder doesn't exist
    let id = h.file(ra, &t.join("a/IMG_1.jpg"), None);
    assert!(move_photos(&h, &[id], &t.join("b/Trip"), true).is_err());
    let rep = move_photos(&h, &[id], &t.join("b/Trip"), false).unwrap();
    assert_eq!((rep.done, rep.queued), (0, 1));
    assert!(t.join("a/IMG_1.jpg").exists(), "the file waits where it is");
    assert_eq!(h.path_of(id).unwrap(), t.join("b/Trip/IMG_1.jpg").to_string_lossy());
    assert!(h.jobs.lock().iter().any(|j| matches!(j, Job::ApplyPending)));
    // The drive comes back: the move happens.
    std::fs::create_dir_all(t.join("b")).unwrap();
    let rep = crate::pending::apply_in(&h.conn.lock()).unwrap();
    assert_eq!(rep.done, 1);
    assert!(t.join("b/Trip/IMG_1.jpg").exists());
    assert!(!t.join("a/IMG_1.jpg").exists());
}

#[test]
fn moving_out_of_the_library_forgets_the_photo() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.join("lib"));
    let id = h.file(r, &t.join("lib/a.jpg"), None);
    std::fs::create_dir_all(t.join("elsewhere")).unwrap();
    let rep = move_photos(&h, &[id], &t.join("elsewhere"), false).unwrap();
    assert!(rep.outside_library);
    assert!(h.path_of(id).is_none());
    assert!(!rep.moved[0].in_library);
    // And back again.
    let back = move_file_back(&h, Path::new(&rep.moved[0].to), &t.join("lib"), "a.jpg").unwrap();
    assert_eq!(back, t.join("lib/a.jpg"));
}

#[test]
fn deleting_one_of_a_pair_keeps_the_shared_sidecar() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    h.file(r, &t.join("IMG_1.CR2"), None);
    let jpg = h.file(r, &t.join("IMG_1.JPG"), None);
    let other = h.file(r, &t.join("IMG_2.JPG"), None);
    h.file(r, &t.join("IMG_1.xmp"), None);
    h.file(r, &t.join("IMG_2.xmp"), None);
    let rep = delete_photos(&h, &[jpg, other], true).unwrap();
    assert_eq!(rep.done, 2);
    assert_eq!(names(&t.0), ["IMG_1.CR2", "IMG_1.xmp"]);
}

#[test]
fn folder_rename_keeps_photos_and_waiting_moves() {
    let t = Tmp::new("fo");
    let h = host();
    let ra = h.root(&t.join("a"));
    h.root(&t.join("b")); // away
    let id = h.file(ra, &t.join("a/Trip/IMG_1.jpg"), None);
    let keep = h.file(ra, &t.join("a/Trip/IMG_2.jpg"), None);
    let lookalike = h.file(ra, &t.join("a/Trip 2/IMG_3.jpg"), None);
    move_photos(&h, &[id], &t.join("b/Out"), false).unwrap();
    let dst = rename_folder(&h, &t.join("a/Trip"), "Iceland").unwrap();
    assert_eq!(dst, t.join("a/Iceland"));
    assert_eq!(h.path_of(keep).unwrap(), t.join("a/Iceland/IMG_2.jpg").to_string_lossy());
    assert_eq!(
        h.path_of(lookalike).unwrap(),
        t.join("a/Trip 2/IMG_3.jpg").to_string_lossy(),
        "a folder whose name only starts the same is left alone"
    );
    // The waiting move now starts from the renamed folder, so it still works.
    assert_eq!(
        crate::pending::source_of(&h.conn.lock(), id).unwrap().unwrap(),
        t.join("a/Iceland/IMG_1.jpg").to_string_lossy()
    );
    std::fs::create_dir_all(t.join("b")).unwrap();
    assert_eq!(crate::pending::apply_in(&h.conn.lock()).unwrap().done, 1);
    assert!(t.join("b/Out/IMG_1.jpg").exists());
}

#[test]
fn folder_rename_refuses_a_taken_name_but_allows_a_case_change() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let id = h.file(r, &t.join("trip/IMG_1.jpg"), None);
    std::fs::create_dir_all(t.join("other")).unwrap();
    assert!(rename_folder(&h, &t.join("trip"), "other").is_err());
    assert!(rename_folder(&h, &t.join("trip"), "a/b").is_err());
    let dst = rename_folder(&h, &t.join("trip"), "Trip").unwrap();
    assert_eq!(names(&t.0), ["Trip", "other"]);
    assert_eq!(h.path_of(id).unwrap(), dst.join("IMG_1.jpg").to_string_lossy());
}

#[test]
fn folder_move_and_copy() {
    let t = Tmp::new("fo");
    let h = host();
    let r = h.root(&t.0);
    let id = h.file(r, &t.join("Trip/IMG_1.jpg"), None);
    h.file(r, &t.join("Trip/sub/IMG_2.jpg"), None);
    std::fs::create_dir_all(t.join("Travel/Trip")).unwrap(); // the name is taken there
    assert!(move_folder(&h, &t.join("Trip"), &t.join("Trip/sub"), false).is_err(), "not into itself");
    let rep = move_folder(&h, &t.join("Trip"), &t.join("Travel"), false).unwrap();
    let dst = rep.folder.unwrap();
    assert_eq!(dst, t.join("Travel/Trip (2)"));
    assert_eq!(h.path_of(id).unwrap(), dst.join("IMG_1.jpg").to_string_lossy());
    assert!(!t.join("Trip").exists());
    let rep = move_folder(&h, &dst, &t.0, true).unwrap();
    let copy = rep.folder.unwrap();
    assert_eq!(names(&copy), ["IMG_1.jpg", "sub"]);
    assert_eq!(names(&dst), ["IMG_1.jpg", "sub"], "the original stays");
}
