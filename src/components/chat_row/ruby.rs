//! The Study view's furigana for a chat row: asking the backend (batched), the cache, and drawing a
//! line with its readings and the emphasis keywords.

use crate::ruby_view::{mark, RubyBatch, RubyCache, RUBY_BATCH_MAX};
use crate::ui_types::RubySpan;
use leptos::prelude::*;
use leptos::reactive::spawn_local;
use leptos::{view, IntoView};
use resonance_types::ipc::AnnotateFurigana;
use std::cell::RefCell;
use std::time::Duration;

/// Lines already given furigana: a row that is built again (a tab switch,
/// paging) does not ask the backend again.
const RUBY_CACHE_LINES: usize = 2000;

/// How long rows wait for each other before the furigana of all of them is asked in one call.
const RUBY_WINDOW: Duration = Duration::from_millis(50);

pub(super) type RubyTarget = WriteSignal<Option<Vec<RubySpan>>>;

thread_local! {
    static RUBY_CACHE: RefCell<RubyCache> = RefCell::new(RubyCache::new(RUBY_CACHE_LINES));
    static RUBY_BATCH: RefCell<RubyBatch<RubyTarget>> = RefCell::new(RubyBatch::new());
}

/// Asks for the furigana of a row's line. Rows built together (a tab switch, paging, turning the
/// Study view on) are answered by one `annotate_furigana` call: the first request opens a short
/// window, and everything asked inside it goes out together.
pub(super) fn ask_ruby(text: &str, target: RubyTarget) {
    let opens = RUBY_BATCH.with(|batch| batch.borrow_mut().add(text, target));
    if opens {
        set_timeout(flush_ruby, RUBY_WINDOW);
    }
}

fn flush_ruby() {
    let calls = RUBY_BATCH.with(|batch| batch.borrow_mut().drain(RUBY_BATCH_MAX));
    for call in calls {
        spawn_local(async move {
            let Ok(lines) = crate::tauri_bridge::call(&AnnotateFurigana {
                texts: call.lines().to_vec(),
            })
            .await
            else {
                return;
            };
            for (line, spans, rows) in call.answer(lines) {
                RUBY_CACHE.with(|cache| cache.borrow_mut().put(&line, spans.clone()));
                // A row that was removed meanwhile is gone: its signal takes nothing.
                for target in rows {
                    let _ = target.try_set(Some(spans.clone()));
                }
            }
        });
    }
}

/// The furigana already given for a line (a row built again does not ask again).
pub(super) fn cached(text: &str) -> Option<Vec<RubySpan>> {
    RUBY_CACHE.with(|cache| cache.borrow().get(text).cloned())
}

/// A message's original: with furigana once the backend has answered, as
/// plain text until then (and for a line without Japanese).
pub(super) fn render_original(
    text: &str,
    spans: Option<Vec<RubySpan>>,
    keywords: &[String],
) -> AnyView {
    match spans {
        Some(spans) => render_ruby(&spans, keywords),
        None => render_emphasized(text, keywords).into_any(),
    }
}

/// Spans as `<ruby>` (reading over the kanji) and plain text, the emphasis
/// keywords marked the way `render_emphasized` marks them.
fn render_ruby(spans: &[RubySpan], keywords: &[String]) -> AnyView {
    mark(spans, keywords)
        .into_iter()
        .map(|piece| {
            let class = if piece.emphasized {
                "text-warning font-black mx-0.5"
            } else {
                ""
            };
            match piece.reading {
                Some(reading) => view! {
                    <ruby class=class>{piece.text}<rt data-reading=reading></rt></ruby>
                }
                .into_any(),
                None => view! { <span class=class>{piece.text}</span> }.into_any(),
            }
        })
        .collect_view()
        .into_any()
}

// CLEANED UP: No more messy text-shadows needed!
pub(super) fn render_emphasized(text: &str, keywords: &[String]) -> impl IntoView {
    if keywords.is_empty() || text.is_empty() {
        return view! { <span>{text.to_string()}</span> }.into_any();
    }

    let mut views = Vec::new();
    let mut current_text = text;

    while !current_text.is_empty() {
        let mut earliest_find = None;
        for kw in keywords {
            if kw.is_empty() {
                continue;
            }
            if let Some(idx) = current_text.find(kw) {
                if earliest_find.map_or(true, |(e_idx, _)| idx < e_idx) {
                    earliest_find = Some((idx, kw));
                }
            }
        }

        match earliest_find {
            Some((idx, kw)) => {
                let before = &current_text[..idx];
                if !before.is_empty() {
                    views.push(
                        view! {
                            <span>{before.to_string()}</span>
                        }
                        .into_any(),
                    );
                }
                // Emphasis keywords keep their warning color, but no shadow needed.
                views.push(
                    view! {
                        <span class="text-warning font-black mx-0.5">
                            {kw.to_string()}
                        </span>
                    }
                    .into_any(),
                );
                current_text = &current_text[idx + kw.len()..];
            }
            None => {
                views.push(
                    view! {
                        <span>{current_text.to_string()}</span>
                    }
                    .into_any(),
                );
                break;
            }
        }
    }

    views.into_view().into_any()
}
