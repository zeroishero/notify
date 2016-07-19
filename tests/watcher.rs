extern crate notify;
extern crate tempdir;
extern crate time;

mod utils;

use notify::*;
use std::sync::mpsc;
use tempdir::TempDir;
use std::thread;
use std::env;

#[cfg(all(feature = "manual_tests", target_os="linux"))]
use std::time::Duration;
#[cfg(all(feature = "manual_tests", target_os="linux"))]
use std::io::prelude::*;
#[cfg(all(feature = "manual_tests", target_os="linux"))]
use std::fs::File;

use utils::*;

#[cfg(target_os="linux")]
#[test]
fn new_inotify() {
    let (tx, _) = mpsc::channel();
    let w: Result<INotifyWatcher> = Watcher::new(tx);
    assert!(w.is_ok());
}

#[cfg(target_os="macos")]
#[test]
fn new_fsevent() {
    let (tx, _) = mpsc::channel();
    let w: Result<FsEventWatcher> = Watcher::new(tx);
    assert!(w.is_ok());
}

#[test]
fn new_null() {
    let (tx, _) = mpsc::channel();
    let w: Result<NullWatcher> = Watcher::new(tx);
    assert!(w.is_ok());
}

#[test]
fn new_poll() {
    let (tx, _) = mpsc::channel();
    let w: Result<PollWatcher> = Watcher::new(tx);
    assert!(w.is_ok());
}

#[test]
fn new_recommended() {
    let (tx, _) = mpsc::channel();
    let w: Result<RecommendedWatcher> = Watcher::new(tx);
    assert!(w.is_ok());
}

// if this test builds, it means RecommendedWatcher is Send.
#[test]
fn test_watcher_send() {
    let (tx, _) = mpsc::channel();

    let mut watcher: RecommendedWatcher = Watcher::new(tx).unwrap();

    thread::spawn(move || {
        watcher.watch(".", RecursiveMode::Recursive).unwrap();
    }).join().unwrap();
}

// if this test builds, it means RecommendedWatcher is Sync.
#[test]
fn test_watcher_sync() {
    use std::sync::{ Arc, RwLock };

    let (tx, _) = mpsc::channel();

    let watcher: RecommendedWatcher = Watcher::new(tx).unwrap();
    let watcher = Arc::new(RwLock::new(watcher));

    thread::spawn(move || {
        let mut watcher = watcher.write().unwrap();
        watcher.watch(".", RecursiveMode::Recursive).unwrap();
    }).join().unwrap();
}

#[test]
fn watch_relative() {
    // both of the following tests set the same environment variable, so they must not run in parallel
    { // watch_relative_directory
        let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");
        tdir.create("dir1");

        env::set_current_dir(tdir.path()).expect("failed to change working directory");

        let (tx, _) = mpsc::channel();
        let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
        watcher.watch("dir1", RecursiveMode::Recursive).expect("failed to watch directory");
    }
    { // watch_relative_file
        let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");
        tdir.create("file1");

        env::set_current_dir(tdir.path()).expect("failed to change working directory");

        let (tx, _) = mpsc::channel();
        let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
        watcher.watch("file1", RecursiveMode::Recursive).expect("failed to watch file");
    }
}

#[test]
#[cfg(all(feature = "manual_tests", target_os="linux"))]
// Test preparation:
// 1. Run `sudo echo 10 > /proc/sys/fs/inotify/max_queued_events`
// 2. Uncomment the lines near "test inotify_queue_overflow" in inotify watcher
fn inotify_queue_overflow() {
    let mut max_queued_events = String::new();
    let mut f = File::open("/proc/sys/fs/inotify/max_queued_events").expect("failed to open max_queued_events");
    f.read_to_string(&mut max_queued_events).expect("failed to read max_queued_events");
    assert_eq!(max_queued_events.trim(), "10");

    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(tdir.mkpath("."), RecursiveMode::Recursive).expect("failed to watch directory");

    for i in 0..20 {
        let filename = format!("file{}", i);
        tdir.create(&filename);
        tdir.remove(&filename);
    }

    sleep(100);

    let deadline = time::precise_time_s() + 5.0;

    let mut rescan_found = false;
    while !rescan_found && time::precise_time_s() < deadline {
        match rx.try_recv() {
            Ok(Event{op: Ok(op::RESCAN), ..}) => rescan_found = true,
            Ok(Event{op: Err(e), ..}) => panic!("unexpected event err: {:?}", e),
            Ok(e) => (),
            Err(mpsc::TryRecvError::Empty) => (),
            Err(e) => panic!("unexpected channel err: {:?}", e)
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(rescan_found);
}

#[test]
fn watch_recursive_create_directory() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(&tdir.mkpath("."), RecursiveMode::Recursive).expect("failed to watch directory");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.create("dir1");
    sleep(10);
    tdir.create("dir1/file1");

    if cfg!(target_os="macos") {
        sleep(100);
    }

    watcher.unwatch(&tdir.mkpath(".")).expect("failed to unwatch directory");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.create("dir1/file2");

    let actual = if cfg!(target_os="windows") {
        // Windows may sneak a write event in there
        let mut events = recv_events(&rx);
        events.retain(|&(_, op, _)| op != op::WRITE);
        events
    } else if cfg!(target_os="macos") {
        inflate_events(recv_events(&rx))
    } else {
        recv_events(&rx)
    };

    assert_eq!(actual, vec![
        (tdir.mkpath("dir1"), op::CREATE, None),
        (tdir.mkpath("dir1/file1"), op::CREATE, None)
    ]);
}

#[test]
fn watch_recursive_move() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "dir1a",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(tdir.mkpath("."), RecursiveMode::Recursive).expect("failed to watch directory");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.create("dir1a/file1");
    tdir.rename("dir1a", "dir1b");
    sleep(10);
    tdir.create("dir1b/file2");

    let actual = if cfg!(target_os="windows") {
        // Windows may sneak a write event in there
        let mut events = recv_events(&rx);
        events.retain(|&(_, op, _)| op != op::WRITE);
        events
    } else if cfg!(target_os="macos") {
        inflate_events(recv_events(&rx))
    } else {
        recv_events(&rx)
    };

    if cfg!(target_os="macos") {
        let cookies = extract_cookies(&actual);
        assert_eq!(cookies.len(), 1);
        assert_eq!(actual, vec![
            (tdir.mkpath("dir1a/file1"), op::CREATE, None),
            (tdir.mkpath("dir1a"), op::CREATE | op::RENAME, Some(cookies[0])), // excessive create event
            (tdir.mkpath("dir1b"), op::RENAME, Some(cookies[0])),
            (tdir.mkpath("dir1b/file2"), op::CREATE, None)
        ]);
    } else {
        let cookies = extract_cookies(&actual);
        assert_eq!(cookies.len(), 1);
        assert_eq!(actual, vec![
            (tdir.mkpath("dir1a/file1"), op::CREATE, None),
            (tdir.mkpath("dir1a"), op::RENAME, Some(cookies[0])),
            (tdir.mkpath("dir1b"), op::RENAME, Some(cookies[0])),
            (tdir.mkpath("dir1b/file2"), op::CREATE, None)
        ]);
    }
}

#[test]
#[ignore]
fn watch_recursive_move_in() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "watch_dir",
        "dir1a/dir1",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(&tdir.mkpath("watch_dir"), RecursiveMode::Recursive).expect("failed to watch directory");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.rename("dir1a", "watch_dir/dir1b");
    sleep(10);
    tdir.create("watch_dir/dir1b/dir1/file1");

    let actual = if cfg!(target_os="windows") {
        // Windows may sneak a write event in there
        let mut events = recv_events(&rx);
        events.retain(|&(_, op, _)| op != op::WRITE);
        events
    } else if cfg!(target_os="macos") {
        inflate_events(recv_events(&rx))
    } else {
        recv_events(&rx)
    };

    if cfg!(target_os="macos") {
        assert_eq!(actual, vec![
            (tdir.mkpath("watch_dir/dir1b"), op::RENAME, None), // fsevent interprets a move_to as a rename event
            (tdir.mkpath("watch_dir/dir1b/dir1/file1"), op::CREATE, None)
        ]);
        panic!("move event should be a create event");
    } else {
        assert_eq!(actual, vec![
            (tdir.mkpath("watch_dir/dir1b"), op::CREATE, None),
            (tdir.mkpath("watch_dir/dir1b/dir1/file1"), op::CREATE, None)
        ]);
    }
}

#[test]
#[ignore]
fn watch_recursive_move_out() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "watch_dir/dir1a/dir1",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(tdir.mkpath("watch_dir"), RecursiveMode::Recursive).expect("failed to watch directory");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.create("watch_dir/dir1a/dir1/file1");
    tdir.rename("watch_dir/dir1a", "dir1b");
    sleep(10);
    tdir.create("dir1b/dir1/file2");

    let actual = if cfg!(target_os="windows") {
        // Windows may sneak a write event in there
        let mut events = recv_events(&rx);
        events.retain(|&(_, op, _)| op != op::WRITE);
        events
    } else if cfg!(target_os="macos") {
        inflate_events(recv_events(&rx))
    } else {
        recv_events(&rx)
    };

    if cfg!(target_os="macos") {
        assert_eq!(actual, vec![
            (tdir.mkpath("watch_dir/dir1a/dir1/file1"), op::CREATE, None),
            (tdir.mkpath("watch_dir/dir1a"), op::CREATE | op::RENAME, None) // excessive create event, fsevent interprets a move_out as a rename event
        ]);
        panic!("move event should be a remove event");
    } else {
        assert_eq!(actual, vec![
            (tdir.mkpath("watch_dir/dir1a/dir1/file1"), op::CREATE, None),
            (tdir.mkpath("watch_dir/dir1a"), op::REMOVE, None),
        ]);
    }
}

#[test]
fn watch_nonrecursive() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "dir1",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(tdir.mkpath("."), RecursiveMode::NonRecursive).expect("failed to watch directory");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.create("dir2");
    sleep(10);
    tdir.create_all(vec![
        "file0",
        "dir1/file1",
        "dir2/file2",
    ]);

    assert_eq!(recv_events(&rx), vec![
        (tdir.mkpath("dir2"), op::CREATE, None),
        (tdir.mkpath("file0"), op::CREATE, None)
    ]);
}

#[test]
fn watch_file() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "file1",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(tdir.mkpath("file1"), RecursiveMode::Recursive).expect("failed to watch directory");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.write("file1");
    tdir.create("file2");

    if cfg!(target_os="macos") {
        assert_eq!(recv_events(&rx), vec![
            (tdir.mkpath("file1"), op::CREATE | op::WRITE, None) // excessive write create
        ]);
    } else {
        assert_eq!(recv_events(&rx), vec![
            (tdir.mkpath("file1"), op::WRITE, None)
        ]);
    }
}

#[test]
fn poll_watch_recursive_create_directory() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    let (tx, rx) = mpsc::channel();
    let mut watcher = PollWatcher::with_delay(tx, 50).expect("failed to create poll watcher");
    watcher.watch(tdir.mkpath("."), RecursiveMode::Recursive).expect("failed to watch directory");

    sleep(1100); // PollWatcher has only a resolution of 1 second

    tdir.create("dir1/file1");

    sleep(1100); // PollWatcher has only a resolution of 1 second

    watcher.unwatch(tdir.mkpath(".")).expect("failed to unwatch directory");

    tdir.create("dir1/file2");

    let mut actual = recv_events(&rx);
    actual.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(actual, vec![
        (tdir.mkpath("."), op::WRITE, None), // parent directory gets modified
        (tdir.mkpath("dir1"), op::CREATE, None),
        (tdir.mkpath("dir1/file1"), op::CREATE, None)
    ]);
}

#[test]
fn poll_watch_recursive_move() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "dir1a",
    ]);

    let (tx, rx) = mpsc::channel();
    let mut watcher = PollWatcher::with_delay(tx, 50).expect("failed to create poll watcher");
    watcher.watch(tdir.mkpath("."), RecursiveMode::Recursive).expect("failed to watch directory");

    sleep(1100); // PollWatcher has only a resolution of 1 second

    tdir.create("dir1a/file1");

    let mut actual = recv_events(&rx);
    actual.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(actual, vec![
        (tdir.mkpath("dir1a"), op::WRITE, None), // parent directory gets modified
        (tdir.mkpath("dir1a/file1"), op::CREATE, None),
    ]);

    sleep(1100); // PollWatcher has only a resolution of 1 second

    tdir.rename("dir1a", "dir1b");
    tdir.create("dir1b/file2");

    actual = recv_events(&rx);
    actual.sort_by(|a, b| a.0.cmp(&b.0));

    if cfg!(target_os="windows") {
        assert_eq!(actual, vec![
            (tdir.mkpath("."), op::WRITE, None), // parent directory gets modified
            (tdir.mkpath("dir1a"), op::REMOVE, None),
            (tdir.mkpath("dir1a/file1"), op::REMOVE, None),
            (tdir.mkpath("dir1b"), op::CREATE, None),
            (tdir.mkpath("dir1b"), op::WRITE, None), // parent directory gets modified
            (tdir.mkpath("dir1b/file1"), op::CREATE, None),
            (tdir.mkpath("dir1b/file2"), op::CREATE, None),
        ]);
    } else {
        assert_eq!(actual, vec![
            (tdir.mkpath("."), op::WRITE, None), // parent directory gets modified
            (tdir.mkpath("dir1a"), op::REMOVE, None),
            (tdir.mkpath("dir1a/file1"), op::REMOVE, None),
            (tdir.mkpath("dir1b"), op::CREATE, None),
            (tdir.mkpath("dir1b/file1"), op::CREATE, None),
            (tdir.mkpath("dir1b/file2"), op::CREATE, None),
        ]);
    }
}

#[test]
fn poll_watch_recursive_move_in() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "watch_dir",
        "dir1a/dir1",
    ]);

    let (tx, rx) = mpsc::channel();
    let mut watcher = PollWatcher::with_delay(tx, 50).expect("failed to create poll watcher");
    watcher.watch(tdir.mkpath("watch_dir"), RecursiveMode::Recursive).expect("failed to watch directory");

    sleep(1100); // PollWatcher has only a resolution of 1 second

    tdir.rename("dir1a", "watch_dir/dir1b");
    tdir.create("watch_dir/dir1b/dir1/file1");

    let mut actual = recv_events(&rx);
    actual.sort_by(|a, b| a.0.cmp(&b.0));

    if cfg!(target_os="windows") {
        assert_eq!(actual, vec![
            (tdir.mkpath("watch_dir"), op::WRITE, None), // parent directory gets modified
            (tdir.mkpath("watch_dir/dir1b"), op::CREATE, None),
            (tdir.mkpath("watch_dir/dir1b/dir1"), op::CREATE, None),
            (tdir.mkpath("watch_dir/dir1b/dir1"), op::WRITE, None), // extra write event
            (tdir.mkpath("watch_dir/dir1b/dir1/file1"), op::CREATE, None),
        ]);
    } else {
        assert_eq!(actual, vec![
            (tdir.mkpath("watch_dir"), op::WRITE, None), // parent directory gets modified
            (tdir.mkpath("watch_dir/dir1b"), op::CREATE, None),
            (tdir.mkpath("watch_dir/dir1b/dir1"), op::CREATE, None),
            (tdir.mkpath("watch_dir/dir1b/dir1/file1"), op::CREATE, None),
        ]);
    }
}

#[test]
fn poll_watch_recursive_move_out() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "watch_dir/dir1a/dir1",
    ]);

    let (tx, rx) = mpsc::channel();
    let mut watcher = PollWatcher::with_delay(tx, 50).expect("failed to create poll watcher");
    watcher.watch(tdir.mkpath("watch_dir"), RecursiveMode::Recursive).expect("failed to watch directory");

    sleep(1100); // PollWatcher has only a resolution of 1 second

    tdir.create("watch_dir/dir1a/dir1/file1");

    let mut actual = recv_events(&rx);
    actual.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(actual, vec![
        (tdir.mkpath("watch_dir/dir1a/dir1"), op::WRITE, None), // parent directory gets modified
        (tdir.mkpath("watch_dir/dir1a/dir1/file1"), op::CREATE, None),
    ]);

    sleep(1100); // PollWatcher has only a resolution of 1 second

    tdir.rename("watch_dir/dir1a", "dir1b");
    tdir.create("dir1b/dir1/file2");

    actual = recv_events(&rx);
    actual.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(actual, vec![
        (tdir.mkpath("watch_dir"), op::WRITE, None), // parent directory gets modified
        (tdir.mkpath("watch_dir/dir1a"), op::REMOVE, None),
        (tdir.mkpath("watch_dir/dir1a/dir1"), op::REMOVE, None),
        (tdir.mkpath("watch_dir/dir1a/dir1/file1"), op::REMOVE, None),
    ]);
}

#[test]
fn poll_watch_nonrecursive() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "dir1",
    ]);

    let (tx, rx) = mpsc::channel();
    let mut watcher = PollWatcher::with_delay(tx, 50).expect("failed to create poll watcher");
    watcher.watch(tdir.mkpath("."), RecursiveMode::NonRecursive).expect("failed to watch directory");

    sleep(1100); // PollWatcher has only a resolution of 1 second

    tdir.create_all(vec![
        "file1",
        "dir1/file1",
        "dir2/file1",
    ]);

    let mut actual = recv_events(&rx);
    actual.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(actual, vec![
        (tdir.mkpath("."), op::WRITE, None), // parent directory gets modified
        (tdir.mkpath("dir1"), op::WRITE, None), // parent directory gets modified
        (tdir.mkpath("dir2"), op::CREATE, None),
        (tdir.mkpath("file1"), op::CREATE, None),
    ]);
}

#[test]
fn poll_watch_file() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "file1",
    ]);

    let (tx, rx) = mpsc::channel();
    let mut watcher = PollWatcher::with_delay(tx, 50).expect("failed to create poll watcher");
    watcher.watch(tdir.mkpath("file1"), RecursiveMode::Recursive).expect("failed to watch directory");

    sleep(1100); // PollWatcher has only a resolution of 1 second

    tdir.write("file1");
    tdir.create("file2");

    assert_eq!(recv_events(&rx), vec![
        (tdir.mkpath("file1"), op::WRITE, None)
    ]);
}

#[test]
#[should_panic]
fn watch_nonexisting() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, _) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(&tdir.mkpath("file1"), RecursiveMode::Recursive).expect("failed to watch file");
}

#[test]
fn unwatch_file() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "file1",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, _) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(&tdir.mkpath("file1"), RecursiveMode::Recursive).expect("failed to watch file");

    match watcher.unwatch(&tdir.mkpath("file1")) {
        Ok(_) => (),
        Err(e) => panic!("{:?}", e),
    }
}

#[test]
fn unwatch_directory() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "dir1",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, _) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(&tdir.mkpath("dir1"), RecursiveMode::Recursive).expect("failed to watch directory");

    match watcher.unwatch(&tdir.mkpath("dir1")) {
        Ok(_) => (),
        Err(e) => panic!("{:?}", e),
    }
}

#[test]
#[cfg(not(target_os="windows"))]
fn unwatch_nonexisting() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, _) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");

    match watcher.unwatch(&tdir.mkpath("file1")) {
        Err(Error::WatchNotFound) => (),
        Err(e) => panic!("{:?}", e),
        Ok(o) => panic!("{:?}", o),
    }
}

#[test]
fn self_delete_file() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "file1",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(&tdir.mkpath("file1"), RecursiveMode::Recursive).expect("failed to watch file");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.remove("file1");

    let actual = if cfg!(target_os="macos") {
        inflate_events(recv_events(&rx))
    } else {
        recv_events(&rx)
    };

    if cfg!(target_os="windows") {
        assert_eq!(actual, vec![
            (tdir.mkpath("file1"), op::REMOVE, None),
        ]);
    } else if cfg!(target_os="macos") {
        assert_eq!(actual, vec![
            (tdir.mkpath("file1"), op::CREATE | op::REMOVE, None),
        ]);
    } else {
        assert_eq!(actual, vec![
            (tdir.mkpath("file1"), op::CHMOD, None),
            (tdir.mkpath("file1"), op::REMOVE, None),
        ]);
    }

    if cfg!(not(any(target_os="windows", target_os="macos"))) {
        match watcher.unwatch(&tdir.mkpath("file1")) {
            Err(Error::WatchNotFound) => (),
            Err(e) => panic!("{:?}", e),
            Ok(o) => panic!("{:?}", o),
        }
    }
}

#[test]
fn self_delete_directory() {
    let tdir = TempDir::new("temp_dir").expect("failed to create temporary directory");

    tdir.create_all(vec![
        "dir1",
    ]);

    if cfg!(target_os="macos") {
        sleep(10);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx).expect("failed to create recommended watcher");
    watcher.watch(&tdir.mkpath("dir1"), RecursiveMode::Recursive).expect("failed to watch directory");

    if cfg!(target_os="windows") {
        sleep(100);
    }

    tdir.remove("dir1");

    let actual = if cfg!(target_os="macos") {
        inflate_events(recv_events(&rx))
    } else {
        recv_events(&rx)
    };

    if cfg!(target_os="windows") {
        assert_eq!(actual, vec![
            (tdir.mkpath("dir1"), Op::empty(), None),
        ]);
    } else if cfg!(target_os="macos") {
        assert_eq!(actual, vec![
            (tdir.mkpath("dir1"), op::CREATE | op::REMOVE, None),
        ]);
    } else {
        assert_eq!(actual, vec![
            (tdir.mkpath("dir1"), op::REMOVE, None),
        ]);
    }

    if cfg!(not(any(target_os="windows", target_os="macos"))) {
        match watcher.unwatch(&tdir.mkpath("dir1")) {
            Err(Error::WatchNotFound) => (),
            Err(e) => panic!("{:?}", e),
            Ok(o) => panic!("{:?}", o),
        }
    }
}