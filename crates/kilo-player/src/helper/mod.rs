//! Helper side: hosts YouTube's player in a hidden system web view.
//! WebView2 (Windows) and WebKitGTK (Linux) hosts come next.

mod mac;

pub use mac::run;
