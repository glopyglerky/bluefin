//! Semantic browser view of Bluefin's existing controller. No credentials or
//! media authority are serialized into markup, attributes, or browser storage.
use super::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Element, Event, HtmlInputElement, HtmlSelectElement};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn button(action: &str, label: &str, disabled: bool) -> String {
    format!(
        "<button type=\"button\" data-action=\"{}\" {}>{}</button>",
        esc(action),
        if disabled { "disabled" } else { "" },
        esc(label)
    )
}
fn field(id: &str, label: &str, kind: &str, value: &str) -> String {
    format!(
        "<label for=\"{id}\">{label}</label><input id=\"{id}\" type=\"{kind}\" value=\"{}\" autocomplete=\"off\">",
        esc(value)
    )
}
fn ready(app: &Bluefin) -> bool {
    app.pending.is_none()
        && app.auth_drains.load(Ordering::Acquire) == 0
        && app.retired.iter().all(|s| s.snapshot().finished)
}
// Reconcile the bounded, Rust-generated tree in place. Native editors and
// activation targets survive unrelated changes; listeners remain delegated.
fn compatible(a: &web_sys::Node, b: &web_sys::Node) -> bool {
    if a.node_type() != b.node_type() || a.node_name() != b.node_name() {
        return false;
    }
    match (a.dyn_ref::<Element>(), b.dyn_ref::<Element>()) {
        (Some(a), Some(b)) => ["id", "data-action", "data-form", "data-choice", "type"]
            .iter()
            .all(|key| a.get_attribute(key) == b.get_attribute(key)),
        _ => true,
    }
}
fn reconcile(old: &web_sys::Node, new: &web_sys::Node, document: &web_sys::Document) {
    if let (Some(a), Some(b)) = (old.dyn_ref::<Element>(), new.dyn_ref::<Element>()) {
        // Input values are properties, never a password attribute. Avoid touching
        // the live editor's value, selection or composition during presentation.
        for key in a.get_attribute_names().iter().filter_map(|v| v.as_string()) {
            if !(key == "value" && a.tag_name() == "INPUT") && !b.has_attribute(&key) {
                let _ = a.remove_attribute(&key);
            }
        }
        for key in b.get_attribute_names().iter().filter_map(|v| v.as_string()) {
            if !(key == "value" && a.tag_name() == "INPUT")
                && a.get_attribute(&key) != b.get_attribute(&key)
            {
                let _ = a.set_attribute(&key, &b.get_attribute(&key).unwrap_or_default());
            }
        }
        if let Some(input) = a.dyn_ref::<HtmlInputElement>() {
            let focused = document.active_element().is_some_and(|e| e == *a);
            if !matches!(input.type_().as_str(), "password" | "file") && !focused {
                let value = b.get_attribute("value").unwrap_or_default();
                if input.value() != value {
                    input.set_value(&value);
                }
            }
            if matches!(input.type_().as_str(), "checkbox" | "radio") {
                input.set_checked(b.has_attribute("checked"));
            }
            return;
        }
    } else {
        if old.node_value() != new.node_value() {
            old.set_node_value(new.node_value().as_deref());
        }
        return;
    }
    // The controller-rendered value is authoritative. Keep the native select
    // mounted and focused, while applying genuine server/model changes.
    let selection = new
        .dyn_ref::<HtmlSelectElement>()
        .map(HtmlSelectElement::value);
    let mut cursor = old.first_child();
    let mut desired = new.first_child();
    while let Some(next) = desired {
        desired = next.next_sibling();
        if let Some(current) = cursor.clone().filter(|c| compatible(c, &next)) {
            reconcile(&current, &next, document);
            cursor = current.next_sibling();
        } else {
            // Insert before the remaining old nodes. A later matching sibling
            // stays mounted (including focused controls); discard leftovers last.
            let mut found = cursor.clone();
            while let Some(candidate) = found.clone() {
                if compatible(&candidate, &next) {
                    break;
                }
                found = candidate.next_sibling();
            }
            if let Some(found) = found {
                while let Some(c) = cursor.clone() {
                    if c == found {
                        break;
                    }
                    cursor = c.next_sibling();
                    let _ = old.remove_child(&c);
                }
                reconcile(&found, &next, document);
                cursor = found.next_sibling();
            } else {
                let _ = old.insert_before(&next, cursor.as_ref());
            }
        }
    }
    while let Some(c) = cursor {
        cursor = c.next_sibling();
        let _ = old.remove_child(&c);
    }
    if let (Some(select), Some(value)) = (old.dyn_ref::<HtmlSelectElement>(), selection)
        && select.value() != value
    {
        select.set_value(&value);
    }
}
fn present(root: &Element, document: &web_sys::Document, markup: &str) {
    if let Ok(next) = document.create_element("div") {
        for key in root
            .get_attribute_names()
            .iter()
            .filter_map(|v| v.as_string())
        {
            let _ = next.set_attribute(&key, &root.get_attribute(&key).unwrap_or_default());
        }
        next.set_inner_html(markup);
        reconcile(root, &next, document);
    }
}
// Account identity stays only in memory. Replacing the account retires all old
// controls, even when an editor had focus and even if the markup is identical.
fn account(app: &Bluefin) -> (bool, Option<String>) {
    (
        app.model.fixture,
        app.model.session.as_ref().map(|s| s.token.clone()),
    )
}
fn artwork_markup(app: &Bluefin, item: &Item, backdrop: bool) -> String {
    app.artwork
        .url(item.id, backdrop)
        .map_or(String::new(), |url| {
            format!(
                "<img src=\"{}\" alt=\"\" decoding=\"async\" loading=\"lazy\">",
                esc(url)
            )
        })
}
fn media_card(app: &Bluefin, item: &Item) -> String {
    let Some(index) = app.model.rows.iter().position(|row| row.id == item.id) else {
        return String::new();
    };
    let mut markup = format!(
        "<article class=\"legacy-card\"><button type=\"button\" data-action=\"detail:{index}\" data-artwork=\"{}:false\">{}<span class=\"legacy-card-caption\"><strong>{}</strong><span>{}</span></span>",
        item.id,
        artwork_markup(app, item, false),
        esc(&item.name),
        esc(item.channel_name.as_deref().unwrap_or(&item.kind))
    );
    if item.progress() > 0.0 {
        markup.push_str(&format!(
            "<progress value=\"{}\" max=\"1\" aria-label=\"Viewing progress\"></progress>",
            item.progress()
        ));
    }
    markup.push_str("</button></article>");
    markup
}
fn featured(app: &Bluefin, item: &Item) -> String {
    let mut markup = format!(
        "<section class=\"legacy-featured\" data-artwork=\"{}:true\">{}<div><h2>{}</h2>",
        item.id,
        artwork_markup(app, item, true),
        esc(&item.name)
    );
    if item.playable() {
        markup.push_str(&button(
            "hero-play",
            if item.progress() > 0.0 {
                "Continue"
            } else {
                "Play"
            },
            false,
        ));
    }
    markup.push_str(&button("hero-detail", "Details", false));
    markup.push_str("</div></section>");
    markup
}
fn search_filters(app: &Bluefin) -> String {
    let mut markup = String::from(
        "<div class=\"legacy-refinements\"><label>Media type<select id=\"search-scope\">",
    );
    for (index, scope) in SearchScope::ALL.iter().enumerate() {
        markup.push_str(&format!(
            "<option value=\"{index}\" {}>{}</option>",
            if *scope == app.filters.scope {
                "selected"
            } else {
                ""
            },
            scope.title()
        ));
    }
    markup.push_str("</select></label>");
    if let Some(facets) = &app.facets {
        for (id, title, options, selected) in [
            (
                "search-studio",
                "Studio",
                &facets.studios,
                app.filters.studio,
            ),
            (
                "search-collection",
                "Collection",
                &facets.collections,
                app.filters.collection,
            ),
        ] {
            markup.push_str(&format!(
                "<label>{title}<select id=\"{id}\"><option value=\"\">All</option>"
            ));
            for option in options {
                markup.push_str(&format!(
                    "<option value=\"{}\" {}>{}</option>",
                    option.id,
                    if Some(option.id) == selected {
                        "selected"
                    } else {
                        ""
                    },
                    esc(&option.name)
                ));
            }
            markup.push_str("</select></label>");
        }
        markup.push_str("<label>Tag<select id=\"search-tag\"><option value=\"\">All tags</option>");
        for tag in &facets.tags {
            markup.push_str(&format!(
                "<option value=\"{}\" {}>{}</option>",
                esc(tag),
                if *tag == app.filters.tag {
                    "selected"
                } else {
                    ""
                },
                esc(tag)
            ));
        }
        markup.push_str("</select></label>");
        if facets.truncated {
            markup.push_str(
                "<p>Showing a limited set of choices. Search still covers the full account.</p>",
            );
        }
        if !facets.warning.is_empty() {
            markup.push_str(&format!("<p role=\"status\">{}</p>", esc(&facets.warning)));
        }
    }
    markup.push_str(&button("clear-filters", "Clear refinements", false));
    markup.push_str(
        "</div><p>Studios, service tags and custom collections come from your server.</p>",
    );
    markup
}
fn view(app: &Bluefin) -> String {
    let connected = app.model.session.is_some() || app.model.fixture;
    let mut h = String::from(
        "<header><a href=\"#content\" class=\"brand\"><svg viewBox=\"200 210 610 640\" width=\"36\" height=\"38\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M225 604C388 534 410 344 620 239C580 399 670 554 790 604C560 554 420 687 225 604Z\"/><path fill=\"none\" stroke=\"currentColor\" stroke-width=\"24\" d=\"M235 724C430 789 575 644 790 712M235 784C430 849 575 704 790 772\"/></svg>Bluefin</a><span>Your library, anywhere</span></header>",
    );
    if connected {
        h.push_str("<nav aria-label=\"Library\">");
        for (n, section) in Section::PRIMARY.into_iter().enumerate() {
            h.push_str(&format!(
                "<button type=\"button\" data-action=\"nav:{n}\" aria-current=\"{}\">{}</button>",
                if section == app.model.section {
                    "page"
                } else {
                    "false"
                },
                Bluefin::section_title(section)
            ));
        }
        h.push_str(
            "</nav><div class=\"legacy-utilities\" role=\"group\" aria-label=\"Library tools\">",
        );
        for (index, section) in Section::UTILITIES.iter().enumerate() {
            h.push_str(&button(&format!("utility:{index}"), section.title(), false));
        }
        h.push_str("</div>");
    }
    h.push_str("<main id=\"content\"><h1>");
    h.push_str(if connected {
        Bluefin::section_title(app.model.section)
    } else {
        "Connect your library"
    });
    h.push_str("</h1>");
    if !connected {
        h.push_str("<section class=\"card login\" aria-label=\"Connect with an invitation\"><h2>Connect with an invitation</h2>");
        if let Some(step) = &app.mesh_step {
            if step.awaiting_approval {
                h.push_str("<p>Your signup request is waiting for administrator approval.</p>");
            }
            if let Some(url) = &step.external_url {
                h.push_str(&format!("<p><a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">Open passkey page</a></p><p>After verifying, return here to finish.</p>", esc(url)));
            }
            let label = if step.awaiting_approval {
                "Check approval"
            } else if step.needs_begin_retry {
                "Retry saved request"
            } else {
                "Verify and finish"
            };
            h.push_str(&button("mesh-proceed", label, !ready(app)));
        } else {
            h.push_str("<form data-form=\"mesh-connect\">");
            h.push_str(&field(
                "mesh-code",
                "Paste a connection code or launch link",
                "password",
                "",
            ));
            if !app.mesh_code.is_empty() {
                h.push_str("<p role=\"status\">An invitation is ready to connect.</p>");
            }
            h.push_str(&field(
                "mesh-name",
                "Username for a signup invitation",
                "text",
                &app.mesh_name,
            ));
            h.push_str(if ready(app) {
                "<button>Connect</button></form>"
            } else {
                "<button disabled>Connect</button></form>"
            });
            h.push_str("<label for=\"mesh-image\">Import QR image or screenshot</label><input id=\"mesh-image\" type=\"file\" accept=\"image/png,image/jpeg\"><label for=\"mesh-camera\">Take a photo of a QR code</label><input id=\"mesh-camera\" type=\"file\" accept=\"image/jpeg,image/png\" capture=\"environment\">");
        }
        h.push_str(&button(
            "mesh-resume",
            "Resume saved connection",
            !ready(app),
        ));
        h.push_str("<p>Passkeys approve this browser. Invitations are used once.</p></section>");
        h.push_str("<section class=\"card login\" aria-label=\"Sign in\">");
        if let Some(request) = &app.device_sign_in {
            h.push_str(&format!(
                "<p>Confirmation code <strong>{}</strong></p>",
                esc(&request.code)
            ));
            if let Some(page) = &request.page {
                h.push_str(&format!("<p><a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">Open passkey page</a></p>", esc(page.as_str())));
            } else {
                h.push_str("<p>Enter this code in a client signed in to your server.</p>");
            }
            h.push_str(&button("device-finish", "Finish sign-in", !ready(app)));
            h.push_str(&button("device-cancel", "Cancel", !ready(app)));
        } else if let Some(server) = &app.connected_server {
            h.push_str(&format!(
                "<h2>{}</h2><form data-form=\"login\">",
                esc(&server.name)
            ));
            h.push_str(&field("name", "Username", "text", &app.name));
            h.push_str(&field("password", "Password", "password", ""));
            h.push_str(if ready(app) {
                "<button>Sign in</button></form>"
            } else {
                "<button disabled>Sign in</button></form>"
            });
            if server.options.passkeys {
                h.push_str(&button(
                    "device-passkey",
                    "Sign in with a passkey",
                    !ready(app),
                ));
            }
            if server.options.quick_connect {
                h.push_str(&button("device-code", "Quick Connect", !ready(app)));
            }
            if server.options.allow_signups {
                h.push_str(&button("signup", "Create account", !ready(app)));
            }
            h.push_str(&button("change-server", "Change server", !ready(app)));
        } else {
            h.push_str("<form data-form=\"connect\">");
            h.push_str(&field("server", "Server address", "url", &app.server));
            h.push_str(if ready(app) {
                "<button>Continue</button></form>"
            } else {
                "<button disabled>Continue</button></form>"
            });
        }
        h.push_str(&button(
            "fixture",
            "Explore original fixtures",
            !ready(app) || app.device_sign_in.is_some(),
        ));
        h.push_str(
            "<p>Original generated test patterns and tones. No licensed media.</p></section>",
        );
    } else {
        if matches!(app.model.section, Section::Search | Section::Discovery) {
            h.push_str("<form data-form=\"search\" class=\"search\">");
            h.push_str(&field(
                "query",
                if app.model.section == Section::Search {
                    "Search all media"
                } else {
                    "Search discovery catalog"
                },
                "search",
                &app.query,
            ));
            h.push_str("<button>Search</button></form>");
            if app.model.section == Section::Search {
                h.push_str(&search_filters(app));
            }
        }
        if app.model.section == Section::Queue {
            h.push_str(&format!(
                "<p>Revision {} · {} tracks</p><ol>",
                app.model.queue.revision,
                app.model.queue.queue.item_ids.len()
            ));
            for (i, id) in app.model.queue.queue.item_ids.iter().enumerate() {
                h.push_str(&format!(
                    "<li>{id} {} {}</li>",
                    button(
                        &format!("queue-play:{i}"),
                        &format!("Play queued track {}", i + 1),
                        false
                    ),
                    button(
                        &format!("remove:{i}"),
                        &format!("Remove queued track {}", i + 1),
                        false
                    )
                ));
            }
            h.push_str("</ol>");
            h.push_str(&format!(
                "<label><input id=\"shuffle\" type=\"checkbox\" {}>Shuffle</label>",
                if app.model.queue.queue.shuffle {
                    "checked"
                } else {
                    ""
                }
            ));
            h.push_str("<label for=\"repeat\">Repeat</label><select id=\"repeat\">");
            for (i, mode) in [
                bluefin_contract::RepeatMode::Off,
                bluefin_contract::RepeatMode::All,
                bluefin_contract::RepeatMode::One,
            ]
            .into_iter()
            .enumerate()
            {
                h.push_str(&format!(
                    "<option value=\"{i}\" {}>{mode:?}</option>",
                    if mode == app.model.queue.queue.repeat {
                        "selected"
                    } else {
                        ""
                    }
                ));
            }
            h.push_str("</select>");
            h.push_str(&button(
                "save",
                "Save queue to account",
                app.pending.is_some()
                    || !app
                        .model
                        .capabilities
                        .as_ref()
                        .is_some_and(|c| c.music_queue_sync),
            ));
            h.push_str(&button(
                "reload",
                "Reload queue, discard local edits",
                false,
            ));
        } else if app.model.section == Section::Discovery {
            if let Some(page) = &app.discovery {
                for (i, item) in page.results.iter().enumerate() {
                    h.push_str(&format!(
                        "<article class=\"card\"><h2>{}</h2><p>{}</p><p>{}</p>",
                        esc(&item.title),
                        esc(&item.overview),
                        esc(&item.availability)
                    ));
                    for (j, action) in item.actions.iter().enumerate() {
                        h.push_str(&button(&format!("discover:{i}:{j}"), &action.label, false));
                    }
                    h.push_str("</article>");
                }
            }
            if let Some(form) = &app.form {
                h.push_str(&format!(
                    "<section class=\"card\"><h2>{}</h2><p>{}</p>",
                    esc(&form.title),
                    esc(&form.notice)
                ));
                for (i, c) in form.choices.iter().enumerate() {
                    h.push_str(&format!(
                        "<label><input type=\"checkbox\" data-choice=\"{i}\" {}>{}</label>",
                        if app.selection.contains(&c.id) {
                            "checked"
                        } else {
                            ""
                        },
                        esc(&c.label)
                    ));
                }
                h.push_str(&format!("<label><input id=\"confirmed\" type=\"checkbox\" {}>I confirm I am authorized to use this media</label>",if app.confirmed {"checked"}else{""}));
                h.push_str(&button(
                    "submit",
                    "Submit server action",
                    app.pending.is_some()
                        || !app.confirmed
                        || (form.requires_selection && app.selection.is_empty()),
                ));
                h.push_str("</section>");
            }
        } else if app.model.section != Section::Account {
            if !app.previous_shelves.is_empty() && app.model.detail.is_none() {
                h.push_str(&button("close", "Back", false));
            }
            if let Some(item) = &app.model.detail {
                h.push_str(&format!(
                    "<section class=\"card detail\"><h2>{}</h2><p>{}</p>",
                    esc(&item.name),
                    esc(&item.overview)
                ));
                if item.playable() {
                    h.push_str(&button("play", "Play", false));
                }
                if item.kind == "Audio" {
                    h.push_str(&button("enqueue", "Add to queue", false));
                }
                if item.folder() {
                    h.push_str(&button("children", "Browse collection", false));
                }
                h.push_str(&button("close", "Back", false));
                h.push_str("</section>");
            }
            if app.model.section == Section::Home
                && app.model.detail.is_none()
                && app.previous_shelves.is_empty()
                && !app.home.shelves.is_empty()
            {
                if let Some(item) = &app.home.featured {
                    h.push_str(&featured(app, item));
                }
                for shelf in &app.home.shelves {
                    h.push_str(&format!(
                        "<section><h2>{}</h2><div class=\"catalog\">",
                        esc(&shelf.title)
                    ));
                    for item in &shelf.items {
                        h.push_str(&media_card(app, item));
                    }
                    h.push_str("</div></section>");
                }
            } else if app.model.section == Section::Search
                && app.model.detail.is_none()
                && app.previous_shelves.is_empty()
            {
                for (index, shelf) in app.search_results.shelves.iter().enumerate() {
                    h.push_str(&format!(
                        "<section><h2>{}</h2><div class=\"catalog\">",
                        shelf.lane.title()
                    ));
                    for item in &shelf.page.items {
                        h.push_str(&media_card(app, item));
                    }
                    h.push_str("</div>");
                    if !shelf.warning.is_empty() {
                        h.push_str(&format!("<p role=\"status\">{}</p>", esc(&shelf.warning)));
                    }
                    if shelf.page.more() {
                        h.push_str(&button(
                            &format!("more:{index}"),
                            "Load more",
                            app.pending.is_some(),
                        ));
                    }
                    h.push_str("</section>");
                }
            } else {
                h.push_str("<div class=\"catalog\">");
                for item in &app.model.rows {
                    h.push_str(&media_card(app, item));
                }
                h.push_str("</div>");
                if app.page.more() {
                    h.push_str(&button("more", "Load more", app.pending.is_some()));
                }
            }
            if app.model.rows.is_empty() && app.pending.is_none() {
                h.push_str(
                    "<p>No media to show. Search by title or artist, or choose a refinement.</p>",
                );
            }
        }
        if app.model.section == Section::Account {
            h.push_str("<section id=\"account\" class=\"card\"><h2>Account</h2>");
            h.push_str(&button("queue", "Music queue", false));
            h.push_str(&button("discovery", "Discover", false));
            h.push_str("<h3>Home personalization</h3><p>Viewing progress and favorites shape Home on your account's server.</p>");
            if app.home.history_generation.is_some() {
                h.push_str("<p>Submitted searches and selected results also help. History stays with this account.</p>");
                h.push_str(&button(
                    "clear-history",
                    "Clear search history",
                    app.pending.is_some(),
                ));
            }
            if app
                .model
                .session
                .as_ref()
                .is_some_and(|session| crate::mesh::is_mesh(&session.token))
            {
                h.push_str("<p>Connected through the encrypted device mesh.</p>");
            } else {
                h.push_str("<p>Connected through HTTPS.</p>");
            }

            if app.model.fixture {
                h.push_str("<p>Original synthetic fixtures</p>");
            }
            if !app.model.fixture {
                h.push_str(&button("mesh-issue", "Connect another device", !ready(app)));
                if let Some(invite) = app.mesh_invite.as_ref().filter(|invite| invite.current()) {
                    h.push_str("<p>Use this once to sign in to your account on another device. Your passkey is still required.</p>");
                    h.push_str(&format!("<img src=\"{}\" alt=\"One-use Bluefin device invitation\" style=\"width:min(100%,340px);height:auto\"><p><a href=\"{}\" download=\"bluefin-invitation.png\">Save QR image</a></p>",esc(&invite.image),esc(&invite.image)));
                    h.push_str(&button("mesh-copy-link", "Copy launch link", !ready(app)));
                    h.push_str(&button("mesh-copy-code", "Copy code", !ready(app)));
                    h.push_str(&format!("<details><summary>Manual connection code</summary><p><code>{}</code></p></details>",esc(&invite.code)));
                }
            }
            h.push_str(&button("logout", "Sign out", false));
            h.push_str("</section>");
        }
    }
    if let Some(service) = &app.playback {
        h.push_str("<section class=\"card\" aria-label=\"Playback settings\"><h2>Playback</h2>");
        h.push_str(&button("stop", "Stop playback", false));
        if let Some(plan) = service.snapshot().plan {
            for (id, label, kind, selected) in [
                ("audio", "Audio stream", "Audio", plan.selection.audio),
                ("subtitle", "Subtitles", "Subtitle", plan.selection.subtitle),
            ] {
                h.push_str(&format!("<label for=\"{id}\">{label}</label><select id=\"{id}\"><option value=\"\">Server default</option>"));
                if kind == "Subtitle" {
                    h.push_str(&format!(
                        "<option value=\"-1\" {}>Off</option>",
                        if selected == Some(-1) { "selected" } else { "" }
                    ));
                }
                for t in plan.tracks.iter().filter(|t| t.kind == kind) {
                    h.push_str(&format!(
                        "<option value=\"{}\" {}>{}</option>",
                        t.index,
                        if selected == Some(t.index) {
                            "selected"
                        } else {
                            ""
                        },
                        esc(&format!(
                            "{} {} {} {}",
                            t.index, t.display_title, t.language, t.codec
                        ))
                    ));
                }
                h.push_str("</select>");
            }
            h.push_str("<p>Native media controls provide play, pause, seek and volume where supported. Subtitles use server negotiation; this protocol supplies no validated WebVTT track URL.</p>");
        }
        h.push_str("</section>");
    }
    if app.pending.is_some() {
        h.push_str("<p role=\"status\">Loading…</p>");
        h.push_str(&button("cancel", "Cancel", false));
    }
    if app.auth_drains.load(Ordering::Acquire) > 0 && app.pending.is_none() {
        h.push_str("<p role=\"status\">Finishing canceled sign-in cleanup</p>");
    }
    for service in &app.retired {
        h.push_str(&format!(
            "<p role=\"status\">{}</p>",
            esc(&service.snapshot().message)
        ));
    }
    if !app.error.is_empty() {
        h.push_str(&format!("<p role=\"alert\">{}</p>", esc(&app.error)));
    }
    h.push_str("</main>");
    h
}

fn dispatch(app: &mut Bluefin, ctx: &egui::Context, action: &str) {
    let (kind, arg) = action.split_once(':').unwrap_or((action, ""));
    let index = arg.parse::<usize>().unwrap_or(usize::MAX);
    match kind {
        "logout" => app.reset(),
        "cancel" => app.cancel(),
        "fixture" if ready(app) => {
            app.reset();
            app.model.fixture = true;
            match protocol::fixture_base() {
                Ok(base) => app.launch_sign_in(ctx, async move {
                    protocol::login(base, "fixture".into(), String::new())
                        .await
                        .map(Outcome::Login)
                }),
                Err(e) => app.error = e,
            }
        }
        "mesh-issue" if ready(app) => app.issue_mesh_invite(ctx),
        "mesh-copy-link" if ready(app) => app.copy_mesh_invite(ctx, true),
        "mesh-copy-code" if ready(app) => app.copy_mesh_invite(ctx, false),
        "mesh-connect" if ready(app) => app.begin_mesh(ctx),
        "mesh-resume" if ready(app) => app.resume_mesh(ctx),
        "mesh-proceed" if ready(app) => app.proceed_mesh(ctx),
        "connect" if ready(app) => app.connect_server(ctx),
        "change-server" if ready(app) => {
            app.connected_server = None;
            app.password.clear();
        }
        "login" if ready(app) && !app.name.trim().is_empty() => app.submit_account(ctx, false),
        "signup" if ready(app) && !app.name.trim().is_empty() => app.submit_account(ctx, true),
        "device-passkey" if ready(app) => app.begin_device_sign_in(ctx, true),
        "device-code" if ready(app) => app.begin_device_sign_in(ctx, false),
        "device-finish" if ready(app) => app.finish_device_sign_in(ctx),
        "device-cancel" if ready(app) => app.cancel_device_sign_in(ctx),
        "nav" => {
            if let Some(section) = Section::PRIMARY.get(index) {
                app.open_section(ctx, *section);
            }
        }
        "utility" => {
            if let Some(section) = Section::UTILITIES.get(index) {
                app.open_section(ctx, *section);
            }
        }
        "queue" => app.open_section(ctx, Section::Queue),
        "discovery" => app.open_section(ctx, Section::Discovery),
        "search" => {
            if app.model.section == Section::Search {
                app.run_search(ctx, true);
            } else {
                app.browse(ctx, None);
            }
        }
        "more" => app.load_more(ctx, if arg.is_empty() { None } else { Some(index) }),
        "clear-filters" => {
            app.filters = SearchFilters::default();
            app.run_search(ctx, false);
        }
        "clear-history" => {
            if let Some(generation) = app.home.history_generation {
                app.call(
                    ctx,
                    "historyClear",
                    "DELETE",
                    "Bluefin/Home/Searches".into(),
                    vec![],
                    Some(serde_json::json!({"generation":generation})),
                );
            }
        }
        "detail" => {
            if let Some(item) = app.model.rows.get(index).cloned() {
                app.open_detail(ctx, item);
            }
        }
        "hero-detail" => {
            if let Some(item) = app.home.featured.clone() {
                app.open_detail(ctx, item);
            }
        }
        "hero-play" => {
            if let Some(item) = app.home.featured.clone() {
                app.play(item);
            }
        }
        "close" => app.back_to_shelf(),
        "play" => {
            if let Some(i) = app.model.detail.clone() {
                app.play(i);
            }
        }
        "children" => app.browse(ctx, app.model.detail.clone()),
        "enqueue" => {
            if let Some(i) = app.model.detail.clone()
                && let Err(e) = app.model.enqueue(&i)
            {
                app.error = e;
            }
        }
        "remove" => {
            if index < app.model.queue.queue.item_ids.len() {
                app.model.queue.queue.item_ids.remove(index);
                app.model.queue.queue.cursor = None;
                app.model.queue_dirty = true;
            }
        }
        "queue-play" => {
            if let Some(id) = app.model.queue.queue.item_ids.get(index) {
                app.play_selected(
                    Item {
                        id: *id,
                        name: "Queued track".into(),
                        kind: "Audio".into(),
                        overview: String::new(),
                        start_date: None,
                        end_date: None,
                        channel_name: None,
                        channel_id: None,
                        ..Default::default()
                    },
                    Selection::default(),
                    Some(index),
                );
            }
        }
        "save"
            if app.pending.is_none()
                && app
                    .model
                    .capabilities
                    .as_ref()
                    .is_some_and(|c| c.music_queue_sync) =>
        {
            let update = UpdateMusicQueue {
                protocol_version: PROTOCOL_VERSION,
                expected_revision: app.model.queue.revision,
                queue: app.model.queue.queue.clone(),
            };
            app.call(
                ctx,
                "queue",
                "PUT",
                "Bluefin/Music/Queue".into(),
                vec![],
                Some(serde_json::to_value(update).unwrap()),
            );
        }
        "reload" => {
            app.model.queue_dirty = false;
            app.browse(ctx, None);
        }
        "discover" => {
            if let Some((i, j)) = arg
                .split_once(':')
                .and_then(|(i, j)| Some((i.parse::<usize>().ok()?, j.parse::<usize>().ok()?)))
                && let Some(a) = app
                    .discovery
                    .as_ref()
                    .and_then(|p| p.results.get(i))
                    .and_then(|r| r.actions.get(j))
            {
                app.call(
                    ctx,
                    "form",
                    "GET",
                    format!("Bluefin/Discovery/Actions/{}", a.id),
                    vec![],
                    None,
                );
            }
        }
        "submit" => {
            if let Some(f) = &app.form
                && app.pending.is_none()
                && app.confirmed
                && (!f.requires_selection || !app.selection.is_empty())
            {
                let body = bluefin_contract::discovery::SubmitAction {
                    action_id: f.id,
                    selections: app.selection.clone(),
                    authorized_use_confirmed: app.confirmed,
                };
                app.call(
                    ctx,
                    "receipt",
                    "POST",
                    "Bluefin/Discovery/Requests".into(),
                    vec![],
                    Some(serde_json::to_value(body).unwrap()),
                );
            }
        }
        "stop" => {
            if let Some(s) = &app.playback {
                s.send(PlaybackCommand::Stop);
            }
        }
        _ => {}
    }
}

pub(crate) fn start() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("Browser unavailable")?;
    let document = window.document().ok_or("Document unavailable")?;
    document
        .get_element_by_id("bluefin")
        .ok_or("Canvas missing")?
        .set_attribute("hidden", "")?;
    let root = document
        .get_element_by_id("semantic")
        .ok_or("Semantic root missing")?;
    root.remove_attribute("hidden")?;
    let app = Rc::new(RefCell::new(Bluefin::default()));
    let ctx = egui::Context::default();
    let last_account = Rc::new(RefCell::new(account(&app.borrow())));
    let last = Rc::new(RefCell::new(view(&app.borrow())));
    root.set_inner_html(&last.borrow());
    for event_name in ["click", "submit", "input", "change"] {
        let app = app.clone();
        let ctx = ctx.clone();
        let root = root.clone();
        let document = document.clone();
        let last = last.clone();
        let last_account = last_account.clone();
        let listener = root.clone();
        let handler = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                return;
            };
            let mut app = app.borrow_mut();
            let id = target.id();
            if matches!(event.type_().as_str(), "input" | "change") {
                if let Some(input) = target.dyn_ref::<HtmlInputElement>() {
                    match id.as_str() {
                        "mesh-code" => app.mesh_code = input.value(),
                        "mesh-name" => app.mesh_name = input.value(),
                        "mesh-image" | "mesh-camera" => {
                            if let Some(file) = input.files().and_then(|files| files.item(0)) {
                                app.import_mesh_image(&ctx, file);
                                input.set_value("");
                            }
                        }
                        "server" => app.server = input.value(),
                        "name" => app.name = input.value(),
                        "password" => app.password = input.value(),
                        "query" => app.query = input.value(),
                        "shuffle" => {
                            app.model.queue.queue.shuffle = input.checked();
                            app.model.queue_dirty = true;
                        }
                        "confirmed" => app.confirmed = input.checked(),
                        _ => {
                            if let Some(i) = target
                                .get_attribute("data-choice")
                                .and_then(|s| s.parse::<usize>().ok())
                                && let Some(c) = app.form.as_ref().and_then(|f| f.choices.get(i))
                            {
                                let id = c.id.clone();
                                app.selection.retain(|v| v != &id);
                                if input.checked() {
                                    app.selection.push(id);
                                }
                            }
                        }
                    }
                }
                if let Some(select) = target.dyn_ref::<HtmlSelectElement>() {
                    let before = app.filters.clone();
                    match id.as_str() {
                        "search-scope" => {
                            if let Some(scope) = select
                                .value()
                                .parse::<usize>()
                                .ok()
                                .and_then(|n| SearchScope::ALL.get(n))
                            {
                                app.filters.scope = *scope;
                            }
                        }
                        "search-studio" => {
                            app.filters.studio = uuid::Uuid::parse_str(&select.value()).ok()
                        }
                        "search-tag" => app.filters.tag = select.value(),
                        "search-collection" => {
                            app.filters.collection = uuid::Uuid::parse_str(&select.value()).ok()
                        }
                        _ => {}
                    }
                    if before != app.filters {
                        app.run_search(&ctx, false);
                    }
                }
                if id == "repeat"
                    && let Some(select) = target.dyn_ref::<HtmlSelectElement>()
                {
                    app.model.queue.queue.repeat = match select.value().as_str() {
                        "1" => bluefin_contract::RepeatMode::All,
                        "2" => bluefin_contract::RepeatMode::One,
                        _ => bluefin_contract::RepeatMode::Off,
                    };
                    app.model.queue_dirty = true;
                }
                if matches!(id.as_str(), "audio" | "subtitle")
                    && let Some(select) = target.dyn_ref::<HtmlSelectElement>()
                    && let Some(service) = &app.playback
                    && let Some(plan) = service.snapshot().plan
                {
                    let mut selection = plan.selection;
                    selection.position = service.snapshot().position;
                    if id == "audio" {
                        selection.audio = select.value().parse().ok();
                    } else {
                        selection.subtitle = select.value().parse().ok();
                    }
                    service.send(PlaybackCommand::Play(Box::new(plan.item), selection, None));
                }
                // Text editing stays entirely native; do not replace focused fields.
                if event.type_() == "input"
                    || matches!(id.as_str(), "server" | "name" | "password" | "query")
                {
                    return;
                }
            } else if event.type_() == "submit" {
                event.prevent_default();
                if let Some(kind) = target.get_attribute("data-form") {
                    dispatch(&mut app, &ctx, &kind);
                }
                if let Some(p) = document
                    .get_element_by_id("password")
                    .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
                {
                    p.set_value("");
                }
            } else if let Ok(Some(button)) = target.closest("[data-action]")
                && let Some(action) = button.get_attribute("data-action")
            {
                dispatch(&mut app, &ctx, &action);
            } else {
                return;
            }
            let next = view(&app);
            if account(&app) != *last_account.borrow() {
                root.set_inner_html("");
                *last_account.borrow_mut() = account(&app);
            }
            present(&root, &document, &next);
            *last.borrow_mut() = next;
        });
        listener.add_event_listener_with_callback(event_name, handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    let status = document
        .get_element_by_id("playback-status")
        .ok_or("Playback status missing")?;
    let mut last_status = String::new();
    let tick = Closure::<dyn FnMut()>::new(move || {
        let mut app = app.borrow_mut();
        app.poll(&ctx);
        let next = view(&app);
        let changed_account = account(&app) != *last_account.borrow();
        if changed_account || next != *last.borrow() {
            if changed_account {
                root.set_inner_html("");
                *last_account.borrow_mut() = account(&app);
            }
            present(&root, &document, &next);
            *last.borrow_mut() = next;
        }
        let text = app
            .playback
            .as_ref()
            .map(|s| {
                let p = s.snapshot();
                format!(
                    "{}{}",
                    p.plan
                        .as_ref()
                        .map(|p| format!("{} · {}. ", p.item.name, p.method))
                        .unwrap_or_default(),
                    p.message
                )
            })
            .unwrap_or_default();
        if text != last_status {
            status.set_text_content(Some(&text));
            last_status = text;
        }
    });
    window.set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        100,
    )?;
    tick.forget();
    Ok(())
}
