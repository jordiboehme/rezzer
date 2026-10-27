// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

mod greetings;

use rezzer_model::Registry;

/// The system allocator with large blocks moved by hand (see `rezzer_limits::Allocator`):
/// on macOS the system's realloc of large blocks leaves memory resident that nothing holds.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

fn main() -> std::process::ExitCode {
    rezzer_cli::main_with(registry(), greetings::after_read)
}

/// The readers of this build, chosen by the `fmt-*` cargo features.
fn registry() -> Registry {
    #[allow(unused_mut)]
    let mut registry = Registry::new();
    #[cfg(feature = "fmt-text")]
    registry.register(Box::new(rezzer_read_text::TextReader));
    #[cfg(feature = "fmt-markdown")]
    registry.register(Box::new(rezzer_read_markdown::MarkdownReader));
    #[cfg(feature = "fmt-csv")]
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    #[cfg(any(feature = "fmt-xlsx", feature = "fmt-ods"))]
    registry.register(Box::new(rezzer_read_sheet::SheetReader));
    #[cfg(feature = "fmt-pdf")]
    registry.register(Box::new(rezzer_read_pdf::PdfReader));
    registry
}
