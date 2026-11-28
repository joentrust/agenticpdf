use super::{
    Reader,
    chrome::{Input, Overlay},
    column::COLUMN_TOP,
    view::CANVAS_PADDING,
};
use crate::ui::pinch::Pinch;
use gpui::{Context, Pixels, Point, ScrollDelta, ScrollWheelEvent, Window, point, px};

pub(super) const MIN_ZOOM: u32 = 25;
pub(super) const MAX_ZOOM: u32 = 400;
const WHEEL_RATE: f32 = 0.01;
const BOTTOM_PADDING: f32 = 96.0;

pub(super) fn wants_zoom(event: &ScrollWheelEvent) -> bool {
    if cfg!(target_os = "macos") {
        event.modifiers.platform
    } else {
        event.modifiers.control
    }
}

pub(super) fn wheel_factor(delta: &ScrollDelta) -> f32 {
    let lines = match delta {
        ScrollDelta::Pixels(pixels) => f32::from(pixels.y),
        ScrollDelta::Lines(lines) => lines.y * 20.0,
    };
    (lines * WHEEL_RATE).exp()
}

struct Frame {
    left: f32,
    width: f32,
}

fn frame(inner: f32, viewport: f32) -> Frame {
    let left = ((viewport - inner) / 2.0).max(CANVAS_PADDING);
    Frame {
        left,
        width: (inner + CANVAS_PADDING * 2.0).max(viewport),
    }
}

impl Reader {
    pub(super) fn zoom_percent(&self) -> Option<f32> {
        let tab = self.tabs.current()?;
        match tab.zoom {
            Some(zoom) => Some(zoom as f32),
            None => self.image.as_ref().map(|image| image.zoom_percent),
        }
    }

    pub(super) fn content_width(&mut self, window: &Window) -> Option<f32> {
        match self.column(window) {
            Some(column) => Some(column.width),
            None => self.image.as_ref().map(|image| image.width),
        }
    }

    pub(super) fn open_zoom_field(&mut self) {
        let percent = self.zoom_percent().map_or(100, |zoom| zoom.round() as u32);
        self.chrome.zoom_input.clear();
        self.chrome.zoom_input.insert(&percent.to_string());
        self.chrome.zoom_input.select_all();
        self.chrome.zoom_invalid = false;
        self.chrome.input = Input::Zoom;
    }

    pub(super) fn submit_zoom_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.chrome.zoom_input.text.parse::<u32>() {
            Ok(zoom) if (MIN_ZOOM..=MAX_ZOOM).contains(&zoom) => {
                self.set_overlay(Overlay::None, cx);
                self.set_zoom(Some(zoom), window, cx);
            }
            _ => {
                self.chrome.zoom_invalid = true;
                self.chrome.zoom_input.select_all();
                cx.notify();
            }
        }
    }

    pub(super) fn drive_zoom_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, step)) = self.zoom_probe.as_ref() else {
            return;
        };
        let step = *step;
        let Some(column) = self.column(window).filter(|_| self.image.is_some()) else {
            window.request_animation_frame();
            return;
        };
        let (top, height) = self.viewport(window);
        let Some(tab) = self.tabs.current() else {
            return;
        };
        let session = tab.session;
        let blank = column.visible(top, height).any(|page| {
            let shown = self
                .image_key
                .as_ref()
                .is_some_and(|key| key.session == session && key.page == page);
            !shown
                && self
                    .pages
                    .find(|key| key.session == session && key.page == page)
                    .is_none()
        });
        let steps = crate::bench::ZOOM_STEPS;
        if let Some((probe, step)) = self.zoom_probe.as_mut() {
            probe.frame(blank);
            *step += 1;
            if *step > steps * 2 {
                probe.report_as("zoom", column.len(), cx);
                self.zoom_probe = None;
                return;
            }
        }
        if step == 0 {
            if let Some(tab) = self.tabs.current_mut() {
                tab.zoom = Some(100);
            }
            self.request(window);
        } else {
            let factor = 4.0f32.powf(1.0 / steps as f32);
            let bounds = self.tabs.current().map(|tab| tab.scroll.bounds());
            if let Some(bounds) = bounds {
                let centre = bounds.center();
                self.zoom_by(
                    if step <= steps { factor } else { 1.0 / factor },
                    centre,
                    window,
                    cx,
                );
            }
        }
        window.request_animation_frame();
    }

    pub(super) fn pinch(&mut self, pinch: Pinch, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_document() || self.chrome.overlay != Overlay::None {
            return;
        }
        let height = f32::from(window.viewport_size().height);
        let pointer = point(px(pinch.x), px(height - pinch.y_from_bottom));
        let inside = self
            .tabs
            .current()
            .is_some_and(|tab| tab.scroll.bounds().contains(&pointer));
        if inside {
            self.zoom_by(1.0 + pinch.magnification, pointer, window, cx);
        }
    }

    pub(super) fn zoom_by(
        &mut self,
        factor: f32,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let Some(current) = self.zoom_percent() else {
            return;
        };
        let Some(session) = self.tabs.current().map(|tab| tab.session) else {
            return;
        };
        let start = match self.chrome.zoom_target {
            Some((known, target)) if known == session && (target.round() - current).abs() < 1.0 => {
                target
            }
            _ => current,
        };
        let target = (start * factor).clamp(MIN_ZOOM as f32, MAX_ZOOM as f32);
        self.chrome.zoom_target = Some((session, target));
        let zoom = target.round() as u32;
        if zoom as f32 == current {
            return;
        }
        self.zoom_around(zoom, current, pointer, window);
        self.request(window);
        cx.notify();
    }

    fn zoom_around(
        &mut self,
        zoom: u32,
        current: f32,
        pointer: Point<Pixels>,
        window: &mut Window,
    ) {
        let Some(tab) = self.tabs.current() else {
            return;
        };
        let bounds = tab.scroll.bounds();
        let viewport = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let offset = tab.scroll.offset();
        let local = (
            f32::from(pointer.x - bounds.origin.x),
            f32::from(pointer.y - bounds.origin.y),
        );
        let content = (local.0 - f32::from(offset.x), local.1 - f32::from(offset.y));
        let before = self.column(window);
        let image = self.image.as_ref().map(|image| (image.width, image.height));
        if let Some(tab) = self.tabs.current_mut() {
            tab.zoom = Some(zoom);
        }
        let after = self.column(window);
        let target = match (before, after) {
            (Some(before), Some(after)) if before.len() == after.len() && before.len() > 0 => {
                let old = frame(before.width, viewport.0);
                let new = frame(after.width, viewport.0);
                let y = content.1 - COLUMN_TOP;
                let page = before
                    .tops
                    .partition_point(|top| *top <= y)
                    .saturating_sub(1)
                    .min(before.len() - 1);
                let across = (content.0 - old.left - before.lefts[page]) / before.widths[page];
                let down = (y - before.tops[page]) / before.heights[page];
                (
                    new.left + after.lefts[page] + across * after.widths[page],
                    COLUMN_TOP + after.tops[page] + down * after.heights[page],
                    new.width,
                    COLUMN_TOP + after.height + BOTTOM_PADDING,
                )
            }
            _ => {
                let Some((width, height)) = image else {
                    return;
                };
                let scale = zoom as f32 / current;
                let old = frame(width, viewport.0);
                let new = frame(width * scale, viewport.0);
                (
                    new.left + (content.0 - old.left) * scale,
                    COLUMN_TOP + (content.1 - COLUMN_TOP) * scale,
                    new.width,
                    COLUMN_TOP + height * scale + BOTTOM_PADDING,
                )
            }
        };
        let (x, y, width, height) = target;
        let limit = |content: f32, viewport: f32| (content - viewport).max(0.0);
        let offset = point(
            px(-(x - local.0).clamp(0.0, limit(width, viewport.0))),
            px(-(y - local.1).clamp(0.0, limit(height, viewport.1))),
        );
        if let Some(tab) = self.tabs.current_mut() {
            tab.scroll.set_offset(offset);
            tab.restore_scroll = None;
            tab.pending_scroll = false;
            tab.follow_scroll = true;
        }
    }
}
