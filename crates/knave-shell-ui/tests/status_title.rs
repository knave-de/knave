use knave_ui::TextWrap;
#[test]
fn focused_status_title_shapes_one_line_and_clips_to_its_allocation() {
    use knave_desktop_api::{
        DesktopSnapshot, WindowId, WindowSummary, WorkspaceId, WorkspaceSummary,
    };
    use knave_ui::toolkit::TextMeasurer;
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        eprintln!("no headless adapter; status-title shaping test skipped");
        return;
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let mut painter =
        knave_renderer::WgpuPainter::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
    for title in [
        "W".repeat(100),
        "文書".repeat(100),
        "first\nsecond\rthird".into(),
    ] {
        let snapshot = DesktopSnapshot {
            generation: 1,
            overview_visible: false,
            workspaces: vec![WorkspaceSummary {
                workspace: WorkspaceId(1),
                active: true,
                window_count: 1,
                visible_window_count: 1,
            }],
            windows: vec![WindowSummary {
                id: WindowId(1),
                workspace: WorkspaceId(1),
                title,
                app_id: String::new(),
                focused: true,
                minimized: false,
                floating: false,
                fullscreen: false,
                maximized: false,
            }],
        };
        let bar = knave_shell_ui::status_bar::StatusBar::new(
            1,
            600.0,
            36.0,
            Some(&snapshot),
            true,
            None,
            &[],
        );
        let (bounds, text) = bar
            .scene
            .nodes()
            .iter()
            .find_map(|node| match node {
                knave_shell_ui::UiNode::Label {
                    id: knave_shell_ui::NodeId(8),
                    bounds,
                    text,
                    ..
                } => Some((*bounds, text)),
                _ => None,
            })
            .unwrap();
        let commands: Vec<_> = bar
            .display
            .commands
            .iter()
            .filter_map(|command| match command {
                knave_ui::DisplayCommand::Text {
                    bounds: b,
                    text: t,
                    style,
                    transform,
                    clip,
                } if *b == bounds && t == text => Some((style, *transform, *clip)),
                _ => None,
            })
            .collect();
        assert_eq!(commands.len(), 1);
        let (style, transform, clip) = commands[0];
        assert_eq!(style.wrap, TextWrap::None);
        assert_eq!(clip, Some(bounds));
        assert!(!text.contains(['\n', '\r']));
        let layout = painter.measure(text, style, bounds.width);
        assert!(layout.size[1] <= style.line_height);
        assert_eq!(transform, knave_ui::Transform2D::IDENTITY);
    }
}
