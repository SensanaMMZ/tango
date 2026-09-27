//! Training-mode session view: the emulator pane plus opponent-view menu
//! and a side-swap toggle that hands the
//! player control of the other core — over the shared corner commands.

use super::*;
use crate::session::training::TrainingSession;
use crate::session::update::training::Message;
use crate::session::Message as SessionMessage;

/// Training: emulator + selected opponent layout + the view/swap control
/// cluster + the shared corner commands.
pub(crate) fn view<'a>(s: &'a TrainingSession, ctx: Ctx<'a>) -> Element<'a, SessionMessage> {
    let Ctx { lang, state, .. } = ctx;
    let now = iced::time::Instant::now();
    let (main_horizontal, main_vertical) = main_frame_alignment(ctx.opponent_view);
    let frame = framebuffer_view(ctx, None, main_horizontal, main_vertical);
    let frame = stacked_framebuffers(ctx, frame, None, ctx.opponent_view);
    let body = emulator_body(ctx, frame, [None, None]);
    let mut stacked = stack![body];
    // Opponent-screen PiP — outside the controls gate, so it doesn't tuck
    // away with the idle cursor (same treatment as replay).
    if ctx.opponent_view == crate::config::OpponentView::PictureInPicture {
        if let Some(o) = pip_overlay(ctx, None) {
            stacked = stacked.push(o);
        }
    }
    // What the CPU is reacting to — outside the controls gate, so it
    // stays up while the battle runs.
    if let Some(o) = cpu_overlay(s) {
        stacked = stacked.push(o);
    }
    if state.controls_anim.visible(now) {
        stacked = stacked.push(bottom_bar(lang, s, state, ctx.opponent_view));
        stacked = stacked.push(corner_commands_overlay(lang, state, SessionMessage::Close, false));
    }
    finish_session_stack(lang, state, stacked)
}

/// A debug readout of exactly what the CPU opponent was handed on its
/// latest tick, top-left over the game: HP, whose custom screen is open,
/// and the game's own summary of its detail (for BN6: forms, Beast Out
/// turns, charge, gauge, and the CPU's own hand while it picks). Shown
/// only while a CPU drives the other side.
fn cpu_overlay<'a>(s: &TrainingSession) -> Option<Element<'a, SessionMessage>> {
    if s.opponent_kind() == tango_session::opponent::Kind::Dummy {
        return None;
    }
    let obs = s.opponent_view()?;
    let mut lines = vec![format!(
        "CPU sees · HP {} / {} · custom {:?}",
        obs.units[0].hp, obs.units[1].hp, obs.custom
    )];
    lines.extend(obs.detail.iter().flatten().map(|d| d.summary()));
    let plate = container(text(lines.join("\n")).size(11).font(iced::Font::MONOSPACE))
        .padding([6, 8])
        .style(hud_chip_plate);
    Some(
        container(plate)
            .width(Fill)
            .height(Fill)
            .padding(8)
            .align_x(iced::alignment::Horizontal::Left)
            .align_y(iced::alignment::Vertical::Top)
            .into(),
    )
}

/// One 32×32 icon toggle, lit (primary text + hairline) while `active` —
/// the same chip treatment the replay transport uses for its display
/// toggles.
fn toggle_button<'a>(icon: Icon, active: bool, label: String, msg: Message) -> Element<'a, Message> {
    let style = lit_plate_button(active);
    iced::widget::tooltip(
        button(
            container(icon.widget().size(16.0))
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0))
                .center(Fill),
        )
        .padding(0)
        .width(Length::Fixed(32.0))
        .height(Length::Fixed(32.0))
        .style(style)
        .on_press(msg),
        widgets::tooltip_bubble(label),
        iced::widget::tooltip::Position::Bottom,
    )
    .gap(4)
    .into()
}

/// The PiP + swap toggles in a floating plate bar, bottom-centered over
/// the emulator and sliding past the bottom edge when the cursor idles —
/// the compact twin of the replay transport bar. Its own hover pin keeps
/// it up while the cursor rests on it.
fn bottom_bar<'a>(
    lang: &'a unic_langid::LanguageIdentifier,
    s: &'a TrainingSession,
    state: &'a State,
    opponent_view: crate::config::OpponentView,
) -> Element<'a, SessionMessage> {
    let now = iced::time::Instant::now();
    let opponent_view_style = lit_plate_button(opponent_view != crate::config::OpponentView::Off);
    let opponent_view_menu = iced::widget::tooltip(
        widgets::MenuButton::new(
            container(opponent_view_icon(opponent_view).widget().size(16.0))
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0))
                .center(Fill),
            opponent_view_items(lang, opponent_view, Message::SetOpponentView, false),
            true,
            [7.0, 7.0],
            crate::ui::style::STANDARD_PADDING,
            opponent_view_style,
        )
        .menu_width(260.0)
        .on_toggle(Message::BarMenuToggled),
        widgets::tooltip_bubble(format!(
            "{}: {}",
            t!(lang, "training-opponent-view"),
            opponent_view_label(lang, opponent_view)
        )),
        iced::widget::tooltip::Position::Bottom,
    )
    .gap(4);
    let bar = row![
        opponent_view_menu,
        toggle_button(
            Icon::ArrowLeftRight,
            s.is_swapped(),
            t!(lang, "training-swap"),
            Message::ToggleSwap
        ),
        toggle_button(
            Icon::Bot,
            s.opponent_kind() != tango_session::opponent::Kind::Dummy,
            t!(lang, "training-cpu"),
            Message::ToggleCpu
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let plate = container(bar).padding([8, 12]).style(hud_chip_plate);
    let mapped: Element<'a, SessionMessage> = Element::from(plate).map(SessionMessage::Training);
    // Hover pin: on_press is a capture sink so a click on the plate
    // between the toggles re-asserts the pin instead of falling through.
    let hover_pin = iced::widget::mouse_area(mapped)
        .on_enter(SessionMessage::ControlsHovered(true))
        .on_exit(SessionMessage::ControlsHovered(false))
        .on_press(SessionMessage::ControlsHovered(true));
    let slid = anim::slide_in(
        hover_pin,
        state.controls_anim.progress(now),
        iced::Vector::new(0.0, CONTROLS_SLIDE),
    );
    container(slid)
        .width(Fill)
        .height(Fill)
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Bottom)
        .padding(12)
        .into()
}
