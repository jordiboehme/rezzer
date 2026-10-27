// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The terminal guard, tested with a fake screen. Nothing here touches the real terminal.

use std::cell::RefCell;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use rezzer_derez::{Guard, Screen};

type Log = Rc<RefCell<Vec<&'static str>>>;

struct Fake {
    log: Log,
    fail_enter: bool,
}

impl Screen for Fake {
    fn enter(&mut self) -> io::Result<()> {
        if self.fail_enter {
            return Err(io::Error::other("no terminal"));
        }
        self.log.borrow_mut().push("enter");
        Ok(())
    }

    fn leave(&mut self) -> io::Result<()> {
        self.log.borrow_mut().push("leave");
        Ok(())
    }
}

fn fake(log: &Log, fail_enter: bool) -> Fake {
    Fake {
        log: Rc::clone(log),
        fail_enter,
    }
}

#[test]
fn guard_leaves_on_drop() {
    let log = Log::default();
    {
        let _guard = Guard::enter(fake(&log, false)).expect("enter");
        assert_eq!(*log.borrow(), ["enter"]);
    }
    assert_eq!(*log.borrow(), ["enter", "leave"]);
}

#[test]
fn guard_leaves_on_panic() {
    let log = Log::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _guard = Guard::enter(fake(&log, false)).expect("enter");
        panic!("a deliberate panic inside the guard");
    }));
    assert!(result.is_err());
    assert_eq!(*log.borrow(), ["enter", "leave"]);
}

#[test]
fn leave_is_idempotent() {
    let log = Log::default();
    {
        let mut guard = Guard::enter(fake(&log, false)).expect("enter");
        guard.leave().expect("first leave");
        guard.leave().expect("second leave");
    }
    assert_eq!(*log.borrow(), ["enter", "leave"]);
}

#[test]
fn failed_enter_gives_no_guard() {
    let log = Log::default();
    let result = Guard::enter(fake(&log, true));
    assert!(result.is_err());
    drop(result);
    assert!(log.borrow().is_empty());
}

#[test]
fn a_failed_leave_is_not_retried() {
    struct FailLeave(Log);
    impl Screen for FailLeave {
        fn enter(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn leave(&mut self) -> io::Result<()> {
            self.0.borrow_mut().push("leave");
            Err(io::Error::other("gone"))
        }
    }
    let log = Log::default();
    {
        let mut guard = Guard::enter(FailLeave(Rc::clone(&log))).expect("enter");
        assert!(guard.leave().is_err());
        assert!(guard.leave().is_ok());
    }
    assert_eq!(*log.borrow(), ["leave"]);
}
