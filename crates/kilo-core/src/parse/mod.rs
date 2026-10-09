//! Turns YouTube Music's responses into `model` types.
//!
//! The JSON is deserialized straight into the narrow structs below: serde
//! skips every field we don't name, so no JSON tree is ever built and only
//! the strings we keep are allocated.

mod raw;

use std::sync::OnceLock;

use crate::model::{Account, Entry, Header, Page, PageKind, Section, Target, Thumb};
use crate::{Error, Result};
use raw::*;

/// The `X-Goog-FieldMask` for "up next" requests: exactly the fields
/// `up_next` reads, built from its structs. A radio comes back as 75 KB of
/// JSON instead of 560 KB (8 KB instead of 36 KB on the wire), no slower.
///
/// Browse and search answers go without one: their masks come out either
/// too long to send (45 KB) or, with wildcards, slow for the server.
pub fn next_mask() -> &'static str {
    static MASK: OnceLock<String> = OnceLock::new();
    MASK.get_or_init(raw::mask::<NextFirst>)
}

/// The mask for more of an endless queue (`Client::next_continuation`).
pub fn next_more_mask() -> &'static str {
    static MASK: OnceLock<String> = OnceLock::new();
    MASK.get_or_init(raw::mask::<NextMore>)
}

/// The mask for account requests: just the name, handle and photo (about
/// 0.5 KB instead of 5.6 KB).
pub fn account_mask() -> &'static str {
    static MASK: OnceLock<String> = OnceLock::new();
    MASK.get_or_init(raw::mask::<AccountResponse>)
}

/// The signed-in account, from an account menu answer.
pub fn account(json: &[u8]) -> Result<Account> {
    let r: AccountResponse = serde_json::from_slice(json).map_err(|e| Error::Parse(e.to_string()))?;
    let h = r
        .actions
        .into_iter()
        .find_map(|a| a.open_popup_action?.popup.multi_page_menu_renderer?.header?.active_account_header_renderer)
        .ok_or_else(|| Error::Parse("no account in response".into()))?;
    Ok(Account {
        name: join(&h.account_name).into(),
        handle: join(&h.channel_handle).into(),
        photo: best(&h.account_photo.thumbnails).map(|t| Thumb::new(&t.url, false)),
    })
}

/// A browse page: home, explore, library, album, playlist, artist, or a
/// continuation of one.
pub fn browse(json: &[u8]) -> Result<Page> {
    let r: BrowseResponse = serde_json::from_slice(json).map_err(|e| Error::Parse(e.to_string()))?;
    let mut page = Page::default();
    if let Some(h) = &r.header {
        page.header = header(h);
    }
    if let Some(c) = r.contents {
        if let Some(single) = c.single_column_browse_results_renderer {
            for tab in single.tabs.into_iter().take(1) {
                if let Some(list) = tab.tab_renderer.content.and_then(|c| c.section_list_renderer) {
                    page.continuation = continuation(&list.continuations);
                    sections(list.contents, &mut page);
                }
            }
        }
        if let Some(two) = c.two_column_browse_results_renderer {
            // Header (and, on some pages, more shelves) on the left…
            for tab in two.tabs.into_iter().take(1) {
                if let Some(list) = tab.tab_renderer.content.and_then(|c| c.section_list_renderer) {
                    sections(list.contents, &mut page);
                }
            }
            // …tracks and suggestions on the right.
            if let Some(list) = two.secondary_contents.and_then(|c| c.section_list_renderer) {
                sections(list.contents, &mut page);
            }
        }
    }
    if let Some(cc) = r.continuation_contents {
        if let Some(list) = cc.section_list_continuation {
            page.continuation = continuation(&list.continuations);
            sections(list.contents, &mut page);
        }
        if let Some(shelf) = cc.music_playlist_shelf_continuation {
            let (entries, more) = list_entries(shelf.contents);
            page.sections.push(Section::List {
                title: "".into(),
                entries,
                continuation: more.or_else(|| continuation(&shelf.continuations)),
            });
        }
    }
    for action in r.on_response_received_actions {
        if let Some(append) = action.append_continuation_items_action {
            let (entries, more) = list_entries(append.continuation_items);
            page.sections.push(Section::List { title: "".into(), entries, continuation: more });
        }
    }
    Ok(page)
}

/// Search results.
pub fn search(json: &[u8]) -> Result<Page> {
    let r: SearchResponse = serde_json::from_slice(json).map_err(|e| Error::Parse(e.to_string()))?;
    let mut page = Page::default();
    let tab = r.contents.and_then(|c| c.tabbed_search_results_renderer).and_then(|t| t.tabs.into_iter().next());
    if let Some(list) = tab.and_then(|t| t.tab_renderer.content).and_then(|c| c.section_list_renderer) {
        sections(list.contents, &mut page);
    }
    Ok(page)
}

/// "Up next": a queue YouTube Music builds (a track's radio, a mix, the rest
/// of a playlist), and the token for more of it when it's endless (radios
/// and mixes are).
#[derive(Debug, Default)]
pub struct UpNext {
    pub entries: Vec<Entry>,
    pub continuation: Option<Box<str>>,
}

/// An "up next" answer, or the continuation of one. Where a music video
/// comes with its song version, the song is taken, as YouTube Music does
/// with Song/Video set to Song: Kilo never shows video, and the song's
/// player streams a still picture instead of video (measured: 41% less
/// data).
pub fn up_next(json: &[u8]) -> Result<UpNext> {
    let r: NextResponse = serde_json::from_slice(json).map_err(|e| Error::Parse(e.to_string()))?;
    let more = r.continuation_contents.and_then(|c| c.playlist_panel_continuation);
    let panel = r
        .contents
        .and_then(|c| c.single_column_music_watch_next_results_renderer)
        .and_then(|c| c.tabbed_renderer.watch_next_tabbed_results_renderer.tabs.into_iter().next())
        .and_then(|t| t.tab_renderer.content)
        .and_then(|c| c.music_queue_renderer)
        .and_then(|q| q.content)
        .and_then(|c| c.playlist_panel_renderer);
    // A continuation answer also carries a (not always empty) first panel:
    // its own part is the one asked for.
    let panel = more.or(panel).ok_or_else(|| Error::Parse("no queue in response".into()))?;
    let continuation = continuation(&panel.continuations);
    let entries = panel
        .contents
        .into_iter()
        .filter_map(|item| {
            let video = match (item.playlist_panel_video_renderer, item.playlist_panel_video_wrapper_renderer) {
                (Some(video), _) => video,
                (None, Some(w)) => {
                    let mut versions: Vec<PanelVideo> = std::iter::once(w.primary_renderer.playlist_panel_video_renderer)
                        .chain(w.counterpart.into_iter().map(|c| c.counterpart_renderer.playlist_panel_video_renderer))
                        .collect();
                    let song =
                        versions.iter().position(|v| video_type(v.navigation_endpoint.as_ref()) == "MUSIC_VIDEO_TYPE_ATV").unwrap_or(0);
                    versions.swap_remove(song)
                }
                (None, None) => return None,
            };
            let kind = video_type(video.navigation_endpoint.as_ref());
            Some(Entry {
                title: join(&video.title).into(),
                subtitle: first_segment(&video.short_byline_text.unwrap_or(video.long_byline_text)).into(),
                // Songs have square art; videos 16:9 frames.
                thumb: best(&video.thumbnail.thumbnails).map(|t| Thumb::new(&t.url, kind != "MUSIC_VIDEO_TYPE_ATV")),
                target: Target::Play { video_id: video.video_id.into(), playlist_id: None, music_video: kind == "MUSIC_VIDEO_TYPE_OMV" },
                duration: join(&video.length_text).into(),
            })
        })
        .collect();
    Ok(UpNext { entries, continuation })
}

fn sections(contents: Vec<SectionItem>, page: &mut Page) {
    for item in contents {
        if let Some(h) = item.music_responsive_header_renderer {
            page.header = Some(responsive_header(&h));
        }
        if let Some(inner) =
            item.music_editable_playlist_detail_header_renderer.and_then(|h| h.header).and_then(|h| h.music_responsive_header_renderer)
        {
            page.header = Some(responsive_header(&inner));
        }
        if let Some(shelf) = item.music_carousel_shelf_renderer.or(item.music_immersive_carousel_shelf_renderer) {
            let title = shelf.header.as_ref().map(shelf_title).unwrap_or_default();
            // Carousels of song rows ("Quick picks") are grids of rows.
            let rows = shelf.contents.iter().filter(|i| i.music_responsive_list_item_renderer.is_some()).count();
            let as_list = rows * 2 > shelf.contents.len();
            let entries: Vec<Entry> = shelf.contents.into_iter().filter_map(entry).collect();
            if !entries.is_empty() {
                page.sections.push(if as_list {
                    Section::Grid { title: title.into(), entries }
                } else {
                    Section::Cards { title: title.into(), entries }
                });
            }
        }
        if let Some(shelf) = item.music_shelf_renderer {
            let (entries, more) = list_entries(shelf.contents);
            page.sections.push(Section::List {
                title: join(&shelf.title).into(),
                entries,
                continuation: more.or_else(|| continuation(&shelf.continuations)),
            });
        }
        if let Some(shelf) = item.music_playlist_shelf_renderer {
            let (entries, more) = list_entries(shelf.contents);
            page.sections.push(Section::List {
                title: "".into(),
                entries,
                continuation: more.or_else(|| continuation(&shelf.continuations)),
            });
        }
        if let Some(grid) = item.grid_renderer {
            let title = grid.header.and_then(|h| h.grid_header_renderer).map(|h| join(&h.title)).unwrap_or_default();
            let entries = grid.items.into_iter().filter_map(entry).collect();
            page.sections.push(Section::Cards { title: title.into(), entries });
        }
        if let Some(card) = item.music_card_shelf_renderer {
            // Search's "top result": the result itself, then its highlights.
            let target = card.title.runs.first().and_then(|r| r.navigation_endpoint.as_ref()).map_or(Target::None, target);
            let top = Entry {
                title: join_linked(&card.title).into(),
                subtitle: join(&card.subtitle).into(),
                thumb: card.thumbnail.music_thumbnail_renderer.as_ref().and_then(thumb_of),
                target,
                duration: "".into(),
            };
            let (mut entries, _) = list_entries(card.contents);
            entries.insert(0, top);
            page.sections.push(Section::List { title: "".into(), entries, continuation: None });
        }
        if let Some(desc) = item.music_description_shelf_renderer {
            page.sections.push(Section::Text { title: join(&desc.header).into(), body: join(&desc.description).into() });
        }
        if let Some(section) = item.item_section_renderer {
            // Search lists results as one item section per row: merge
            // consecutive rows into a single list.
            let mut rows = Vec::new();
            for c in section.contents {
                if let Some(shelf) = c.music_shelf_renderer {
                    let (entries, _) = list_entries(shelf.contents);
                    page.sections.push(Section::List { title: join(&shelf.title).into(), entries, continuation: None });
                }
                let item = ShelfItem { music_responsive_list_item_renderer: c.music_responsive_list_item_renderer, ..Default::default() };
                rows.extend(entry(item));
            }
            if !rows.is_empty() {
                match page.sections.last_mut() {
                    Some(Section::List { entries, .. }) => entries.extend(rows),
                    _ => page.sections.push(Section::List { title: "".into(), entries: rows, continuation: None }),
                }
            }
        }
    }
}

fn list_entries(items: Vec<ShelfItem>) -> (Vec<Entry>, Option<Box<str>>) {
    let mut more = None;
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        if let Some(c) = &item.continuation_item_renderer {
            more = c.continuation_endpoint.continuation_command.as_ref().map(|c| c.token.clone().into());
            continue;
        }
        if let Some(e) = entry(item) {
            entries.push(e);
        }
    }
    (entries, more)
}

fn entry(item: ShelfItem) -> Option<Entry> {
    if let Some(r) = item.music_two_row_item_renderer {
        let wide = r.aspect_ratio.contains("16_9");
        return Some(Entry {
            title: join(&r.title).into(),
            subtitle: join(&r.subtitle).into(),
            thumb: r.thumbnail_renderer.music_thumbnail_renderer.as_ref().and_then(thumb_of).map(|mut t| {
                t.wide = wide;
                t
            }),
            target: r.navigation_endpoint.as_ref().map_or(Target::None, target),
            duration: "".into(),
        });
    }
    if let Some(r) = item.music_responsive_list_item_renderer {
        let mut columns = r.flex_columns.iter().map(|c| &c.music_responsive_list_item_flex_column_renderer.text);
        let title = columns.next().map(join_linked).unwrap_or_default();
        let subtitle = columns.map(join_linked).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" • ");
        let first_run_target = r
            .flex_columns
            .first()
            .and_then(|c| c.music_responsive_list_item_flex_column_renderer.text.runs.first())
            .and_then(|run| run.navigation_endpoint.as_ref())
            .map(target);
        let target = if let Some(video_id) = r.playlist_item_data.as_ref().map(|p| p.video_id.clone()).filter(|v| !v.is_empty()) {
            let (playlist_id, music_video) = match &first_run_target {
                Some(Target::Play { playlist_id, music_video, .. }) => (playlist_id.clone(), *music_video),
                _ => (None, false),
            };
            Target::Play { video_id: video_id.into(), playlist_id, music_video }
        } else if let Some(nav) = &r.navigation_endpoint {
            target(nav)
        } else {
            first_run_target.unwrap_or(Target::None)
        };
        let duration = r.fixed_columns.first().map(|c| join(&c.music_responsive_list_item_fixed_column_renderer.text)).unwrap_or_default();
        return Some(Entry {
            title: title.into(),
            subtitle: subtitle.into(),
            thumb: r.thumbnail.music_thumbnail_renderer.as_ref().and_then(thumb_of),
            target,
            duration: duration.into(),
        });
    }
    if let Some(r) = item.music_navigation_button_renderer {
        return Some(Entry {
            title: join(&r.button_text).into(),
            subtitle: "".into(),
            thumb: None,
            target: r.click_command.as_ref().map_or(Target::None, target),
            duration: "".into(),
        });
    }
    if let Some(r) = item.music_multi_row_list_item_renderer {
        return Some(Entry {
            title: join(&r.title).into(),
            subtitle: join(&r.subtitle).into(),
            thumb: r.thumbnail.music_thumbnail_renderer.as_ref().and_then(thumb_of),
            target: r.on_tap.as_ref().map_or(Target::None, target),
            duration: "".into(),
        });
    }
    None
}

fn target(e: &Endpoint) -> Target {
    if let Some(w) = &e.watch_endpoint {
        return Target::Play {
            video_id: w.video_id.clone().into(),
            playlist_id: w.playlist_id.clone().map(Into::into),
            music_video: video_type(Some(e)) == "MUSIC_VIDEO_TYPE_OMV",
        };
    }
    if let Some(w) = &e.watch_playlist_endpoint {
        return Target::PlayPlaylist { playlist_id: w.playlist_id.clone().into() };
    }
    if let Some(b) = &e.browse_endpoint {
        let page_type = b
            .browse_endpoint_context_supported_configs
            .as_ref()
            .and_then(|c| c.browse_endpoint_context_music_config.as_ref())
            .map_or("", |c| c.page_type.as_str());
        let kind = match page_type {
            "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => PageKind::Album,
            "MUSIC_PAGE_TYPE_PLAYLIST" | "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE" => PageKind::Playlist,
            "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" | "MUSIC_PAGE_TYPE_LIBRARY_ARTIST" => PageKind::Artist,
            _ => PageKind::Other,
        };
        return Target::Browse { id: b.browse_id.clone().into(), params: b.params.clone().map(Into::into), kind };
    }
    Target::None
}

/// `MUSIC_VIDEO_TYPE_ATV` (a song), `_OMV` (an official music video),
/// `_UGC`…, or "" when not given.
fn video_type(e: Option<&Endpoint>) -> &str {
    e.and_then(|e| e.watch_endpoint.as_ref())
        .and_then(|w| w.watch_endpoint_music_supported_configs.as_ref())
        .and_then(|c| c.watch_endpoint_music_config.as_ref())
        .map_or("", |c| c.music_video_type.as_str())
}

fn header(h: &HeaderRenderers) -> Option<Header> {
    if let Some(r) = &h.music_immersive_header_renderer {
        return Some(Header {
            title: join(&r.title).into(),
            subtitle: join(&r.monthly_listener_count).into(),
            detail: "".into(),
            thumb: r.thumbnail.music_thumbnail_renderer.as_ref().and_then(thumb_of).map(|mut t| {
                t.wide = true;
                t
            }),
            description: join(&r.description).into(),
        });
    }
    if let Some(r) = &h.music_visual_header_renderer {
        return Some(Header {
            title: join(&r.title).into(),
            subtitle: "".into(),
            detail: "".into(),
            thumb: r.thumbnail.music_thumbnail_renderer.as_ref().and_then(thumb_of),
            description: "".into(),
        });
    }
    if let Some(r) = &h.music_responsive_header_renderer {
        return Some(responsive_header(r));
    }
    None
}

fn responsive_header(r: &ResponsiveHeader) -> Header {
    let mut detail = join(&r.strapline_text_one);
    let second = join(&r.second_subtitle);
    if !second.is_empty() {
        if !detail.is_empty() {
            detail.push_str(" • ");
        }
        detail.push_str(&second);
    }
    Header {
        title: join(&r.title).into(),
        subtitle: join(&r.subtitle).into(),
        detail: detail.into(),
        thumb: r.thumbnail.music_thumbnail_renderer.as_ref().and_then(thumb_of),
        description: r
            .description
            .as_ref()
            .and_then(|d| d.music_description_shelf_renderer.as_ref())
            .map(|d| join(&d.description))
            .unwrap_or_default()
            .into(),
    }
}

fn shelf_title(h: &CarouselHeader) -> String {
    h.music_carousel_shelf_basic_header_renderer.as_ref().map(|b| join(&b.title)).unwrap_or_default()
}

/// The thumbnail, 16:9 when it's at least that wide (a video's): square
/// slots then ask for enough pixels to crop it sharply.
fn thumb_of(t: &MusicThumbnail) -> Option<Thumb> {
    // In u64: the sizes are the server's, and no product of two u32s
    // overflows it.
    best(&t.thumbnail.thumbnails).map(|t| Thumb::new(&t.url, t.height > 0 && u64::from(t.width) * 10 >= u64::from(t.height) * 16))
}

/// The largest thumbnail; `Thumb::sized` rescales it as needed.
fn best(list: &[ThumbnailEntry]) -> Option<&ThumbnailEntry> {
    list.iter().max_by_key(|t| t.width)
}

fn join(t: &Text) -> String {
    t.runs.iter().map(|r| r.text.as_str()).collect()
}

fn join_linked(t: &LinkText) -> String {
    t.runs.iter().map(|r| r.text.as_str()).collect()
}

/// "Artist • Album • 2020" → "Artist".
fn first_segment(t: &Text) -> String {
    let all = join(t);
    all.split(" • ").next().unwrap_or("").to_owned()
}

fn continuation(list: &[Continuation]) -> Option<Box<str>> {
    list.iter()
        .find_map(|c| c.next_continuation_data.as_ref().or(c.next_radio_continuation_data.as_ref()))
        .map(|d| d.continuation.clone().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_carousel_of_albums() {
        let json = br#"{"contents":{"singleColumnBrowseResultsRenderer":{"tabs":[{"tabRenderer":{"content":{"sectionListRenderer":{
            "contents":[{"musicCarouselShelfRenderer":{
                "header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Listen again"}]}}},
                "contents":[{"musicTwoRowItemRenderer":{
                    "title":{"runs":[{"text":"Abbey Road"}]},
                    "subtitle":{"runs":[{"text":"Album"},{"text":" \u2022 "},{"text":"1969"}]},
                    "thumbnailRenderer":{"musicThumbnailRenderer":{"thumbnail":{"thumbnails":[
                        {"url":"https://lh3.googleusercontent.com/a=w60-h60","width":60,"height":60},
                        {"url":"https://lh3.googleusercontent.com/a=w226-h226","width":226,"height":226}]}}},
                    "navigationEndpoint":{"browseEndpoint":{"browseId":"MPREb_x","browseEndpointContextSupportedConfigs":
                        {"browseEndpointContextMusicConfig":{"pageType":"MUSIC_PAGE_TYPE_ALBUM"}}}},
                    "unknownField":{"deeply":[1,2,3]}}}]}}],
            "continuations":[{"nextContinuationData":{"continuation":"TOKEN"}}]}}}}]}}}"#;
        let page = browse(json).unwrap();
        assert_eq!(page.continuation.as_deref(), Some("TOKEN"));
        let Section::Cards { title, entries } = &page.sections[0] else { panic!("expected cards") };
        assert_eq!(&**title, "Listen again");
        assert_eq!(&*entries[0].subtitle, "Album \u{2022} 1969");
        assert_eq!(entries[0].target, Target::Browse { id: "MPREb_x".into(), params: None, kind: PageKind::Album });
        assert_eq!(entries[0].thumb.as_ref().unwrap().sized(100), "https://lh3.googleusercontent.com/a=w100-h100-l75-rj");
    }

    #[test]
    fn carousels_of_song_rows_are_grids() {
        let row = r#"{"musicResponsiveListItemRenderer":{"flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Song",
            "navigationEndpoint":{"watchEndpoint":{"videoId":"45cYwDMibGo"}}}]}}}],"playlistItemData":{"videoId":"45cYwDMibGo"}}}"#;
        let json = String::from(
            r#"{"contents":{"singleColumnBrowseResultsRenderer":{"tabs":[{"tabRenderer":{"content":{"sectionListRenderer":{"contents":[
                {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Quick picks"}]}}},
                "contents":["#,
        ) + row
            + ","
            + row
            + r#"]}}]}}}}]}}}"#;
        let page = browse(json.as_bytes()).unwrap();
        assert!(matches!(&page.sections[0], Section::Grid { title, entries } if &**title == "Quick picks" && entries.len() == 2));
    }

    #[test]
    fn parses_album_tracks() {
        let json = br#"{"contents":{"twoColumnBrowseResultsRenderer":{
            "tabs":[{"tabRenderer":{"content":{"sectionListRenderer":{"contents":[{"musicResponsiveHeaderRenderer":{
                "title":{"runs":[{"text":"Abbey Road"}]},"straplineTextOne":{"runs":[{"text":"The Beatles"}]},
                "secondSubtitle":{"runs":[{"text":"17 songs"}]}}}]}}}}],
            "secondaryContents":{"sectionListRenderer":{"contents":[{"musicShelfRenderer":{"contents":[
                {"musicResponsiveListItemRenderer":{
                    "flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Come Together",
                        "navigationEndpoint":{"watchEndpoint":{"videoId":"45cYwDMibGo","playlistId":"OLAK5"}}}]}}}],
                    "fixedColumns":[{"musicResponsiveListItemFixedColumnRenderer":{"text":{"runs":[{"text":"4:20"}]}}}],
                    "playlistItemData":{"videoId":"45cYwDMibGo"}}}]}}]}}}}}"#;
        let page = browse(json).unwrap();
        let header = page.header.unwrap();
        assert_eq!((&*header.title, &*header.detail), ("Abbey Road", "The Beatles • 17 songs"));
        let entry = &page.sections[0].entries()[0];
        assert_eq!(entry.video_id(), Some("45cYwDMibGo"));
        assert_eq!(&*entry.duration, "4:20");
        assert_eq!(entry.target, Target::Play { video_id: "45cYwDMibGo".into(), playlist_id: Some("OLAK5".into()), music_video: false });
    }

    #[test]
    fn up_next_takes_the_song_version_of_a_music_video() {
        let video = |id: &str, kind: &str| {
            format!(
                r#"{{"playlistPanelVideoRenderer":{{"title":{{"runs":[{{"text":"Song"}}]}},"videoId":"{id}",
                "navigationEndpoint":{{"watchEndpoint":{{"videoId":"{id}","watchEndpointMusicSupportedConfigs":
                    {{"watchEndpointMusicConfig":{{"musicVideoType":"{kind}"}}}}}}}}}}}}"#
            )
        };
        let json = format!(
            r#"{{"contents":{{"singleColumnMusicWatchNextResultsRenderer":{{"tabbedRenderer":{{"watchNextTabbedResultsRenderer":{{"tabs":[
                {{"tabRenderer":{{"content":{{"musicQueueRenderer":{{"content":{{"playlistPanelRenderer":{{"contents":[
                    {{"playlistPanelVideoWrapperRenderer":{{"primaryRenderer":{omv},"counterpart":[{{"counterpartRenderer":{atv}}}]}}}},
                    {other}]}}}}}}}}}}}}]}}}}}}}}}}"#,
            omv = video("dQw4w9WgXcQ", "MUSIC_VIDEO_TYPE_OMV"),
            atv = video("lYBUbBu4W08", "MUSIC_VIDEO_TYPE_ATV"),
            other = video("djV11Xbc914", "MUSIC_VIDEO_TYPE_OMV"),
        );
        let queue = up_next(json.as_bytes()).unwrap().entries;
        assert_eq!(queue[0].video_id(), Some("lYBUbBu4W08"));
        assert!(!queue[0].is_music_video());
        // A music video without a song version stays as it is.
        assert_eq!(queue[1].video_id(), Some("djV11Xbc914"));
        assert!(queue[1].is_music_video());
    }

    #[test]
    fn more_of_a_mix_comes_with_the_token_for_the_next_part() {
        let json = br#"{"continuationContents":{"playlistPanelContinuation":{"contents":[
            {"playlistPanelVideoRenderer":{"videoId":"lYBUbBu4W08","title":{"runs":[{"text":"Song"}]}}}],
            "continuations":[{"nextRadioContinuationData":{"continuation":"MORE"}}]}}}"#;
        let up = up_next(json).unwrap();
        assert_eq!(up.entries.len(), 1);
        assert_eq!(up.continuation.as_deref(), Some("MORE"));
    }

    #[test]
    fn wide_thumbnails_are_marked_wide() {
        let thumb = |w: u32, h: u32| {
            let t: MusicThumbnail = serde_json::from_str(&format!(
                r#"{{"thumbnail":{{"thumbnails":[{{"url":"https://i.ytimg.com/vi/x/hq.jpg","width":{w},"height":{h}}}]}}}}"#
            ))
            .unwrap();
            thumb_of(&t).unwrap().wide
        };
        assert!(thumb(400, 225));
        assert!(!thumb(226, 226));
        // No height given: square, as before.
        assert!(!thumb(400, 0));
        // Absurd sizes from the server don't overflow.
        assert!(thumb(u32::MAX, 1));
        assert!(!thumb(1, u32::MAX));
    }

    #[test]
    fn parses_the_account() {
        let json = br#"{"actions":[{"openPopupAction":{"popup":{"multiPageMenuRenderer":{"header":{"activeAccountHeaderRenderer":{
            "accountName":{"runs":[{"text":"Ada"}]},"channelHandle":{"runs":[{"text":"@ada"}]},
            "accountPhoto":{"thumbnails":[{"url":"https://yt3.ggpht.com/a=s88","width":88}]}}}}}}}]}"#;
        let a = account(json).unwrap();
        assert_eq!((&*a.name, &*a.handle), ("Ada", "@ada"));
        assert_eq!(
            account_mask(),
            "actions(openPopupAction(popup(multiPageMenuRenderer(header(activeAccountHeaderRenderer(accountName(runs(text)),channelHandle(runs(text)),accountPhoto(thumbnails(url,width,height))))))))"
        );
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(browse(b"{not json").is_err());
        assert!(browse(b"{}").unwrap().sections.is_empty());
    }
}

#[cfg(test)]
mod mask_tests {
    #[test]
    fn next_mask_is_compact_and_well_formed() {
        let mask = super::next_mask();
        assert!(mask.len() < 2048, "{} bytes", mask.len());
        assert_eq!(mask.matches('(').count(), mask.matches(')').count());
        assert!(mask.starts_with("contents(singleColumnMusicWatchNextResultsRenderer("));
        assert!(mask.contains("counterpartRenderer(playlistPanelVideoRenderer("));
        let more = super::next_more_mask();
        assert!(more.len() < 2048, "{} bytes", more.len());
        assert!(more.starts_with("continuationContents(playlistPanelContinuation(contents("));
    }
}
