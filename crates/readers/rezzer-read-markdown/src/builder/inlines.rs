// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Inline content: text, breaks, math and the stack of inline frames.

use rezzer_model::{Inline, Loss, LossReason, Severity, plain_text};

use super::{Builder, Frame, InlineFrame, Leaf};

impl<'b> Builder<'b> {
    pub(super) fn on_text(&mut self, s: &str) {
        match &mut self.leaf {
            Leaf::Code { text, .. } | Leaf::Html(text) => {
                text.push_str(s);
            }
            _ => {
                self.ensure_inline_leaf();
                self.push_inline(Inline::Text {
                    text: s.to_string(),
                    emphasis: self.emphasis > 0,
                    strong: self.strong > 0,
                    strike: self.strike > 0,
                });
            }
        }
    }

    /// True in the first paragraph of a plain quote that starts like a callout header, before
    /// its first line break.
    pub(super) fn can_mark_callout_break(&self) -> bool {
        matches!(self.leaf, Leaf::Paragraph)
            && matches!(
                self.stack.last(),
                Some(Frame::Quote { kind: None, blocks, .. }) if blocks.is_empty()
            )
            && matches!(
                self.cur.first(),
                Some(Inline::Text { text, .. }) if text.starts_with("[!")
            )
            && self.para_break.is_none()
            && self.inline_stack.is_empty()
    }

    /// A soft break reads as a space. In the first paragraph of a plain quote its position is
    /// kept as a separate inline, so a callout header (`> [!NOTE] Title`) can be told from the
    /// body; `close_top_frame` joins it again when the quote is no callout.
    pub(super) fn soft_break(&mut self) {
        self.ensure_inline_leaf();
        let space = Inline::plain(" ");
        if self.can_mark_callout_break() {
            self.para_break = Some(self.cur.len());
            self.cur.push(space);
        } else {
            self.push_inline(Inline::Text {
                text: " ".into(),
                emphasis: self.emphasis > 0,
                strong: self.strong > 0,
                strike: self.strike > 0,
            });
        }
    }

    pub(super) fn ensure_inline_leaf(&mut self) {
        if matches!(self.leaf, Leaf::None) {
            self.leaf = Leaf::Implicit;
        }
    }

    /// Charges one model element. False when the element budget ran out with this one: the
    /// element is dropped and reading stops. Once stopped, the frames still open are closed
    /// without charge (their number is bounded by the depth limit).
    pub(super) fn charge(&mut self) -> bool {
        if self.elements_stopped {
            return true;
        }
        if self.budget.add_elements(1).is_err() {
            self.elements_stopped = true;
            return false;
        }
        true
    }

    /// Pushes a formula. An empty one has nothing to show and is not kept, but it is said:
    /// the source had math delimiters with nothing inside.
    pub(super) fn math_or_loss(&mut self, latex: String) {
        if !latex.trim().is_empty() {
            self.push_inline(Inline::Math { latex });
            return;
        }
        self.empty_math += 1;
        if self.empty_math == 1 {
            self.empty_math_loss = Some(self.losses.len());
            self.need_math_line = true;
            self.losses.push(Loss {
                location: "document".into(),
                what: "empty math (a formula with no content) dropped".into(),
                why: LossReason::Uncertain,
                severity: Severity::Warning,
            });
        }
    }

    pub(super) fn push_inline(&mut self, inline: Inline) {
        if !self.charge() {
            return;
        }
        self.display_math = false;
        match self.inline_stack.last_mut() {
            Some(InlineFrame::Image { alt, .. }) => alt.push_str(&plain_text(&[inline])),
            Some(InlineFrame::Link { inlines, .. }) => push_merged(inlines, inline),
            None => push_merged(&mut self.cur, inline),
        }
    }

    pub(super) fn end_inline_frame(&mut self) {
        match self.inline_stack.pop() {
            Some(InlineFrame::Link {
                target,
                title,
                internal,
                wiki,
                inlines,
            }) => {
                self.record_link(&target, internal);
                self.push_inline(Inline::Link {
                    target,
                    title,
                    internal,
                    wiki,
                    inlines,
                });
            }
            Some(InlineFrame::Image { target, title, alt }) => {
                self.record_image(&target, &alt);
                self.push_inline(Inline::Image {
                    target,
                    alt,
                    title: Some(title).filter(|t| !t.is_empty()),
                });
            }
            None => {}
        }
    }

    pub(super) fn close_inline_frames(&mut self) {
        while !self.inline_stack.is_empty() {
            self.end_inline_frame();
        }
    }
}

/// Appends an inline, joining plain text of the same style with the previous one.
pub(super) fn push_merged(inlines: &mut Vec<Inline>, inline: Inline) {
    if let Inline::Text {
        text,
        emphasis,
        strong,
        strike,
    } = &inline
        && let Some(Inline::Text {
            text: prev,
            emphasis: pe,
            strong: ps,
            strike: pk,
        }) = inlines.last_mut()
        && pe == emphasis
        && ps == strong
        && pk == strike
    {
        prev.push_str(text);
        return;
    }
    inlines.push(inline);
}
