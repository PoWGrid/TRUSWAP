pub mod api;
pub mod chains;
pub mod config;
pub mod core;
pub mod relay;
pub mod runner;
pub mod swap;
pub mod tru_p2p;
pub mod tui;
pub mod wallet;
pub mod ndk_camera;

pub mod android;

#[cfg(all(target_os = "android", target_arch = "aarch64"))]
std::arch::global_asm!(
    r#"
    .section .tdata,"awT",@progbits
    .balign 64
    .globl bionic_tls_anchor
    bionic_tls_anchor:
        .quad 0
    "#
);


pub mod base_watcher {
    pub use crate::chains::evm::*;
}
pub mod crypto {
    pub use crate::core::*;
}
pub mod orderbook {
    pub use crate::swap::orderbook::*;
}
pub mod swap_engine {
    pub use crate::swap::engine::*;
}
pub mod ws_codec {
    pub use crate::api::codec::*;
}
pub mod server {
    pub use crate::api::*;
}
pub mod storage {
    pub use crate::wallet::keystore::*;
}
pub mod tru_node {
    pub use crate::wallet::vault::*;
}
