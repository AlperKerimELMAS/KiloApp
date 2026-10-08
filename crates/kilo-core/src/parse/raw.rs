//! The slice of YouTube Music's response format that Kilo reads. Every
//! struct defaults missing fields; unknown fields are skipped by serde.
//!
//! The same structs spell the field mask sent with "up next" requests
//! (`Fields`), so the server returns exactly the fields parsed here.

use serde::Deserialize;

/// Writes a type's part of a field mask: `(field,field(sub),…)` for
/// structs, nothing for plain values.
pub trait Fields {
    fn fields(_out: &mut String) {}
}

impl Fields for String {}
impl Fields for u32 {}

impl<T: Fields> Fields for Vec<T> {
    fn fields(out: &mut String) {
        T::fields(out);
    }
}

impl<T: Fields> Fields for Option<T> {
    fn fields(out: &mut String) {
        T::fields(out);
    }
}

/// The `X-Goog-FieldMask` asking for exactly the fields of `T`.
pub fn mask<T: Fields>() -> String {
    let mut out = String::new();
    T::fields(&mut out);
    // The top level goes without the outer parentheses.
    out[1..out.len() - 1].to_owned()
}

/// `snake_case` → `camelCase`, as serde renames the fields.
fn camel(snake: &str, out: &mut String) {
    let mut upper = false;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
        } else {
            out.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        }
    }
}

macro_rules! raw {
    ($($(#[$m:meta])* struct $name:ident { $($field:ident : $ty:ty),* $(,)? })*) => {$(
        $(#[$m])*
        #[derive(Clone, Debug, Default, Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        pub struct $name { $(pub $field: $ty),* }

        impl Fields for $name {
            fn fields(out: &mut String) {
                out.push('(');
                $(
                    camel(stringify!($field), out);
                    <$ty as Fields>::fields(out);
                    out.push(',');
                )*
                out.pop();
                out.push(')');
            }
        }
    )*};
}

raw! {
    struct BrowseResponse {
        header: Option<HeaderRenderers>,
        contents: Option<Contents>,
        continuation_contents: Option<ContinuationContents>,
        on_response_received_actions: Vec<ResponseAction>,
    }
    struct Contents {
        single_column_browse_results_renderer: Option<Tabs>,
        two_column_browse_results_renderer: Option<TwoColumn>,
    }
    struct Tabs { tabs: Vec<Tab> }
    struct TwoColumn { tabs: Vec<Tab>, secondary_contents: Option<TabContent> }
    struct Tab { tab_renderer: TabRenderer }
    struct TabRenderer { content: Option<TabContent> }
    struct TabContent { section_list_renderer: Option<SectionList> }
    struct SectionList { contents: Vec<SectionItem>, continuations: Vec<Continuation> }

    struct ContinuationContents {
        section_list_continuation: Option<SectionList>,
        music_playlist_shelf_continuation: Option<Shelf>,
    }
    struct ResponseAction { append_continuation_items_action: Option<AppendItems> }
    struct AppendItems { continuation_items: Vec<ShelfItem> }

    struct SectionItem {
        music_carousel_shelf_renderer: Option<Carousel>,
        music_immersive_carousel_shelf_renderer: Option<Carousel>,
        music_shelf_renderer: Option<Shelf>,
        music_playlist_shelf_renderer: Option<Shelf>,
        grid_renderer: Option<Grid>,
        music_card_shelf_renderer: Option<CardShelf>,
        music_description_shelf_renderer: Option<DescriptionShelf>,
        music_responsive_header_renderer: Option<ResponsiveHeader>,
        music_editable_playlist_detail_header_renderer: Option<EditableHeader>,
        item_section_renderer: Option<ItemSection>,
    }
    struct Carousel { header: Option<CarouselHeader>, contents: Vec<ShelfItem> }
    struct CarouselHeader { music_carousel_shelf_basic_header_renderer: Option<BasicHeader> }
    struct BasicHeader { title: Text }
    struct Shelf { title: Text, contents: Vec<ShelfItem>, continuations: Vec<Continuation> }
    struct Grid { header: Option<GridHeader>, items: Vec<ShelfItem> }
    struct GridHeader { grid_header_renderer: Option<BasicHeader> }
    struct CardShelf { title: LinkText, subtitle: Text, thumbnail: ThumbnailHolder, contents: Vec<ShelfItem> }
    struct DescriptionShelf { header: Text, description: Text }
    struct ItemSection { contents: Vec<ItemSectionEntry> }
    struct ItemSectionEntry {
        music_responsive_list_item_renderer: Option<ListItem>,
        music_shelf_renderer: Option<Shelf>,
    }

    struct ShelfItem {
        music_two_row_item_renderer: Option<TwoRowItem>,
        music_responsive_list_item_renderer: Option<ListItem>,
        music_navigation_button_renderer: Option<NavigationButton>,
        music_multi_row_list_item_renderer: Option<MultiRowItem>,
        continuation_item_renderer: Option<ContinuationItem>,
    }
    struct TwoRowItem {
        title: Text,
        subtitle: Text,
        thumbnail_renderer: ThumbnailHolder,
        navigation_endpoint: Option<Endpoint>,
        aspect_ratio: String,
    }
    struct ListItem {
        flex_columns: Vec<FlexColumn>,
        fixed_columns: Vec<FixedColumn>,
        thumbnail: ThumbnailHolder,
        navigation_endpoint: Option<Endpoint>,
        playlist_item_data: Option<PlaylistItemData>,
    }
    struct FlexColumn { music_responsive_list_item_flex_column_renderer: LinkColumnText }
    struct FixedColumn { music_responsive_list_item_fixed_column_renderer: ColumnText }
    struct LinkColumnText { text: LinkText }
    struct ColumnText { text: Text }
    struct PlaylistItemData { video_id: String }
    struct NavigationButton { button_text: Text, click_command: Option<Endpoint> }
    struct MultiRowItem { title: Text, subtitle: Text, thumbnail: ThumbnailHolder, on_tap: Option<Endpoint> }
    struct ContinuationItem { continuation_endpoint: ContinuationEndpoint }
    struct ContinuationEndpoint { continuation_command: Option<ContinuationCommand> }
    struct ContinuationCommand { token: String }

    struct HeaderRenderers {
        music_immersive_header_renderer: Option<ImmersiveHeader>,
        music_visual_header_renderer: Option<VisualHeader>,
        music_responsive_header_renderer: Option<ResponsiveHeader>,
    }
    struct ImmersiveHeader { title: Text, description: Text, thumbnail: ThumbnailHolder, monthly_listener_count: Text }
    struct VisualHeader { title: Text, thumbnail: ThumbnailHolder }
    struct ResponsiveHeader {
        title: Text,
        subtitle: Text,
        strapline_text_one: Text,
        second_subtitle: Text,
        thumbnail: ThumbnailHolder,
        description: Option<DescriptionHolder>,
    }
    struct DescriptionHolder { music_description_shelf_renderer: Option<DescriptionShelf> }
    struct EditableHeader { header: Option<EditableInner> }
    struct EditableInner { music_responsive_header_renderer: Option<ResponsiveHeader> }

    struct Text { runs: Vec<Run> }
    struct Run { text: String }
    /// Text whose first run may link somewhere.
    struct LinkText { runs: Vec<LinkRun> }
    struct LinkRun { text: String, navigation_endpoint: Option<Endpoint> }
    struct Endpoint {
        browse_endpoint: Option<BrowseEndpoint>,
        watch_endpoint: Option<WatchEndpoint>,
        watch_playlist_endpoint: Option<WatchPlaylistEndpoint>,
    }
    struct BrowseEndpoint {
        browse_id: String,
        params: Option<String>,
        browse_endpoint_context_supported_configs: Option<BrowseContext>,
    }
    struct BrowseContext { browse_endpoint_context_music_config: Option<MusicConfig> }
    struct MusicConfig { page_type: String }
    struct WatchEndpoint {
        video_id: String,
        playlist_id: Option<String>,
        watch_endpoint_music_supported_configs: Option<WatchConfigs>,
    }
    struct WatchConfigs { watch_endpoint_music_config: Option<WatchMusicConfig> }
    struct WatchMusicConfig { music_video_type: String }
    struct WatchPlaylistEndpoint { playlist_id: String }

    struct ThumbnailHolder { music_thumbnail_renderer: Option<MusicThumbnail> }
    struct MusicThumbnail { thumbnail: ThumbnailList }
    struct ThumbnailList { thumbnails: Vec<ThumbnailEntry> }
    struct ThumbnailEntry { url: String, width: u32, height: u32 }

    struct Continuation {
        next_continuation_data: Option<ContinuationData>,
        next_radio_continuation_data: Option<ContinuationData>,
    }
    struct ContinuationData { continuation: String }

    struct SearchResponse { contents: Option<SearchContents> }
    struct SearchContents { tabbed_search_results_renderer: Option<Tabs> }

    struct AccountResponse { actions: Vec<AccountAction> }
    struct AccountAction { open_popup_action: Option<OpenPopup> }
    struct OpenPopup { popup: Popup }
    struct Popup { multi_page_menu_renderer: Option<MultiPageMenu> }
    struct MultiPageMenu { header: Option<MenuHeader> }
    struct MenuHeader { active_account_header_renderer: Option<AccountHeader> }
    struct AccountHeader { account_name: Text, channel_handle: Text, account_photo: ThumbnailList }

    struct NextResponse { contents: Option<NextContents>, continuation_contents: Option<NextContinuation> }
    /// The fields of a first "up next" answer (its field mask).
    struct NextFirst { contents: Option<NextContents> }
    /// The fields of a continuation (its field mask).
    struct NextMore { continuation_contents: Option<NextContinuation> }
    /// More of an endless queue (a mix, a radio).
    struct NextContinuation { playlist_panel_continuation: Option<PlaylistPanel> }
    struct NextContents { single_column_music_watch_next_results_renderer: Option<WatchNext> }
    struct WatchNext { tabbed_renderer: TabbedRenderer }
    struct TabbedRenderer { watch_next_tabbed_results_renderer: NextTabs }
    struct NextTabs { tabs: Vec<NextTab> }
    struct NextTab { tab_renderer: NextTabRenderer }
    struct NextTabRenderer { content: Option<NextTabContent> }
    struct NextTabContent { music_queue_renderer: Option<QueueRenderer> }
    struct QueueRenderer { content: Option<QueueContent> }
    struct QueueContent { playlist_panel_renderer: Option<PlaylistPanel> }
    struct PlaylistPanel { contents: Vec<PanelItem>, continuations: Vec<Continuation> }
    struct PanelItem {
        playlist_panel_video_renderer: Option<PanelVideo>,
        playlist_panel_video_wrapper_renderer: Option<PanelWrapper>,
    }
    struct PanelWrapper { primary_renderer: PanelPrimary, counterpart: Vec<Counterpart> }
    struct Counterpart { counterpart_renderer: PanelPrimary }
    struct PanelPrimary { playlist_panel_video_renderer: PanelVideo }
    struct PanelVideo {
        title: Text,
        long_byline_text: Text,
        short_byline_text: Option<Text>,
        length_text: Text,
        video_id: String,
        thumbnail: ThumbnailList,
        navigation_endpoint: Option<Endpoint>,
    }
}
