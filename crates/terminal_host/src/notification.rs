//! A small pop-up window in the screen corner (like the agent panel's), shown when a terminal
//! program becomes blocked or done while Fyx is in the background.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, Context, PlatformDisplay, Size, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions, point,
};
use terminal_ext::osc::StatusState;
use terminal_view::TerminalView;
use ui::{Render, prelude::*};

/// A pop-up closes itself after this long.
const AUTO_CLOSE: Duration = Duration::from_secs(30);

pub(crate) struct SessionNotification {
    title: SharedString,
    caption: SharedString,
    state: StatusState,
    view: WeakEntity<TerminalView>,
    main_window: AnyWindowHandle,
}

impl SessionNotification {
    pub(crate) fn new(
        title: String,
        caption: String,
        state: StatusState,
        view: WeakEntity<TerminalView>,
        main_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(AUTO_CLOSE).await;
            cx.update(|window, _| window.remove_window()).ok();
        })
        .detach();
        Self {
            title: title.into(),
            caption: caption.into(),
            state,
            view,
            main_window,
        }
    }

    pub(crate) fn window_options(screen: Rc<dyn PlatformDisplay>, cx: &App) -> WindowOptions {
        let size = Size {
            width: px(450.),
            height: px(72.),
        };
        let bounds = gpui::Bounds::<Pixels> {
            origin: screen.bounds().top_right() - point(size.width + px(16.), px(-48.)),
            size,
        };
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            display_id: Some(screen.id()),
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(
                release_channel::ReleaseChannel::global(cx)
                    .app_id()
                    .to_owned(),
            ),
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        }
    }
}

impl Render for SessionNotification {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_font = theme_settings::setup_ui_font(window, cx);
        let (icon, color) = match self.state {
            StatusState::Blocked => (IconName::Warning, Color::Warning),
            _ => (IconName::Check, Color::Success),
        };
        h_flex()
            .id("fyx-terminal-notification")
            .size_full()
            .p_3()
            .gap_4()
            .justify_between()
            .elevation_3(cx)
            .text_ui(cx)
            .font(ui_font)
            .border_color(cx.theme().colors().border)
            .rounded_xl()
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .flex_1()
                    .min_w_0()
                    .child(Icon::new(icon).color(color).size(IconSize::Small))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(cx.theme().colors().text)
                                    .truncate()
                                    .child(self.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(cx.theme().colors().text_muted)
                                    .truncate()
                                    .child(self.caption.clone()),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("open", "View")
                            .style(ButtonStyle::Tinted(ui::TintColor::Accent))
                            .full_width()
                            .on_click(cx.listener(|this, _, window, cx| {
                                let (main_window, view) = (this.main_window, this.view.clone());
                                cx.activate(true);
                                cx.defer(move |cx| {
                                    crate::sessions::reveal_in_window(main_window, view, cx)
                                });
                                window.remove_window();
                            })),
                    )
                    .child(
                        Button::new("dismiss", "Dismiss")
                            .full_width()
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}
