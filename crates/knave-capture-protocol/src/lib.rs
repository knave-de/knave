//! Private workspace source v1. Villain owns capture policy; portal clients own consent.

#[cfg(feature = "client")]
pub mod client {
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols::ext::image_capture_source::v1::client::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols::ext::image_capture_source::v1::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/knave-workspace-capture-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocol/knave-workspace-capture-v1.xml");
}

#[cfg(feature = "server")]
pub mod server {
    use wayland_protocols::ext::image_capture_source::v1::server::*;
    use wayland_server;
    use wayland_server::protocol::*;
    pub mod __interfaces {
        use wayland_protocols::ext::image_capture_source::v1::server::__interfaces::*;
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/knave-workspace-capture-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_server_code!("./protocol/knave-workspace-capture-v1.xml");
}
