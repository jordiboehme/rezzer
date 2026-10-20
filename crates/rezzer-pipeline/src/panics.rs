// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Panic containment with the panic location.
//!
//! [`contained`] runs a pipeline stage (read, enrich, render) inside
//! [`rezzer_limits::contain`]. A panic hook, installed once, records the location of a panic
//! on a thread that is inside [`contained`] and prints nothing: the panic becomes an
//! internal-error loss with message and location. Panics anywhere else go to the previous
//! hook unchanged.

use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::sync::Once;

thread_local! {
    /// `Some` while this thread runs a contained stage; holds the last panic location.
    static CAPTURE: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
}

static INSTALL: Once = Once::new();

fn install() {
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let captured = CAPTURE
                .try_with(|c| match c.try_borrow_mut() {
                    Ok(mut slot) => match slot.as_mut() {
                        Some(location) => {
                            // The last panic counts: an earlier one may have been caught
                            // inside the stage.
                            *location = info.location().map(|l| l.to_string());
                            true
                        }
                        None => false,
                    },
                    Err(_) => false,
                })
                .unwrap_or(false);
            if !captured {
                previous(info);
            }
        }));
    });
}

/// Runs `f`; a panic becomes `Err("<message> (at <file:line:column>)")`.
pub(crate) fn contained<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    install();
    let outer = CAPTURE.with(|c| c.borrow_mut().replace(None));
    let result = rezzer_limits::contain(AssertUnwindSafe(f));
    let location = CAPTURE
        .with(|c| std::mem::replace(&mut *c.borrow_mut(), outer))
        .flatten();
    result.map_err(|message| match location {
        Some(at) => format!("{message} (at {at})"),
        None => message,
    })
}
