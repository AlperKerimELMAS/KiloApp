//! Kilo's own words, in English and Turkish, for every front end. Each front
//! end picks the language (the user's choice or the system's) with
//! `set_language`; YouTube's content follows it through the `hl` it asks
//! for.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Turkish,
}

static TURKISH: AtomicBool = AtomicBool::new(false);

pub fn set_language(language: Language) {
    TURKISH.store(language == Language::Turkish, Ordering::Relaxed);
}

macro_rules! strings {
    ($($key:ident => $en:literal, $tr:literal;)*) => {
        /// A piece of Kilo's text.
        #[derive(Clone, Copy, Debug)]
        pub enum S { $($key),* }

        /// `s` in the current language.
        pub fn t(s: S) -> &'static str {
            if TURKISH.load(Ordering::Relaxed) {
                match s { $(S::$key => $tr),* }
            } else {
                match s { $(S::$key => $en),* }
            }
        }
    };
}

strings! {
    Home => "Home", "Ana Sayfa";
    Explore => "Explore", "Keşfet";
    Library => "Library", "Kitaplık";
    SearchPlaceholder => "Search songs, albums, artists, podcasts", "Şarkı, albüm, sanatçı, podcast ara";
    NothingPlaying => "Nothing playing", "Çalan bir şey yok";
    Play => "Play", "Çal";
    Shuffle => "Shuffle", "Karıştır";
    Welcome => "Welcome to Kilo", "Kilo'ya hoş geldiniz";
    WelcomeBody => "Sign in with your YouTube Music Premium account to start listening.", "Dinlemeye başlamak için YouTube Music Premium hesabınızla oturum açın.";
    SignInWithGoogle => "Sign in with Google", "Google ile oturum aç";
    SignInNote => "You sign in on Google's own page. Kilo never sees your password.", "Google'ın kendi sayfasında oturum açarsınız. Kilo şifrenizi asla görmez.";
    SigningIn => "Signing in…", "Oturum açılıyor…";
    SigningInBody => "Finish signing in in the window that opened.", "Açılan pencerede oturum açmayı tamamlayın.";
    SignInWindow => "Sign in to YouTube Music", "YouTube Music'te oturum açın";
    Cancel => "Cancel", "Vazgeç";
    SomethingWrong => "Something went wrong", "Bir sorun oluştu";
    TryAgain => "Try again", "Tekrar dene";
    CantReach => "Couldn't reach YouTube Music. Check your connection.", "YouTube Music'e ulaşılamadı. Bağlantınızı kontrol edin.";
    LoadFailed => "Couldn't load this page. Check your connection.", "Bu sayfa yüklenemedi. Bağlantınızı kontrol edin.";
    PremiumNeeded => "Playback needs a YouTube Music Premium account.", "Çalmak için YouTube Music Premium hesabı gerekir.";
    PlayerFailed => "Couldn't start the player", "Oynatıcı başlatılamadı";
    SigningOut => "Signing out…", "Oturum kapatılıyor…";
    SignOutFailed => "Couldn't sign out completely", "Oturum tamamen kapatılamadı";
    SignOutFailedBody => "Kilo couldn't delete all of its web data. Choose Sign Out again to retry.", "Kilo web verilerinin tamamını silemedi. Yeniden denemek için Oturumu Kapat'ı seçin.";
    SignIn => "Sign In", "Oturum Aç";
    SignOut => "Sign Out", "Oturumu Kapat";
    NotSignedIn => "Not signed in", "Oturum açılmadı";
    NothingHere => "Nothing here", "Burada bir şey yok";
    NothingHereBody => "YouTube Music has nothing to show on this page.", "YouTube Music'in bu sayfada gösterecek bir şeyi yok.";
    Account => "Account", "Hesap";
    Appearance => "Appearance", "Görünüm";
    System => "System", "Sistem";
    Light => "Light", "Açık";
    Dark => "Dark", "Koyu";
    Language => "Language", "Dil";
    AboutKilo => "About Kilo", "Kilo Hakkında";
    HideKilo => "Hide Kilo", "Kilo'yu Gizle";
    HideOthers => "Hide Others", "Diğerlerini Gizle";
    ShowAll => "Show All", "Tümünü Göster";
    QuitKilo => "Quit Kilo", "Kilo'dan Çık";
    Edit => "Edit", "Düzen";
    Undo => "Undo", "Geri Al";
    Redo => "Redo", "Yinele";
    Cut => "Cut", "Kes";
    Copy => "Copy", "Kopyala";
    Paste => "Paste", "Yapıştır";
    SelectAll => "Select All", "Tümünü Seç";
    Search => "Search", "Ara";
    View => "View", "Görüntü";
    Playback => "Playback", "Oynatma";
    PlayPause => "Play/Pause", "Çal/Duraklat";
    Next => "Next", "Sonraki";
    Previous => "Previous", "Önceki";
    Back => "Back", "Geri";
    Window => "Window", "Pencere";
    Minimize => "Minimize", "Küçült";
    Close => "Close", "Kapat";
    SeekForward => "Seek Forward", "İleri Sar";
    SeekBackward => "Seek Backward", "Geri Sar";
    VolumeUp => "Volume Up", "Sesi Yükselt";
    VolumeDown => "Volume Down", "Sesi Alçalt";
    Mute => "Mute", "Sesi Kapat";
    Reload => "Reload Page", "Sayfayı Yenile";
    CollapseSidebar => "Collapse Sidebar", "Kenar Çubuğunu Daralt";
    ExpandSidebar => "Expand Sidebar", "Kenar Çubuğunu Genişlet";
}
