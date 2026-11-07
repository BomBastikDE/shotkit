use super::*;
use shotkit::shapes::{CalloutData, Color, ShapeKind};

/// Find the uppermost callout under the pointer without holding an editor borrow
/// across a modal dialog.
fn hit_callout(editor: &EditorController, point: ShapePoint) -> Option<(usize, CalloutData)> {
    editor
        .shapes()
        .iter()
        .enumerate()
        .rev()
        .find(|(_, shape)| shape.hit_test(point, 6.0))
        .and_then(|(index, shape)| match &shape.kind {
            ShapeKind::Callout { data } => Some((index, data.as_ref().clone())),
            _ => None,
        })
}

// SAFETY CONTRACT:
// Voraussetzung: h ist ein lebendes Editorfenster auf dem UI-Thread.
// Warum erfüllt: Der Maus-Handler ruft die Funktion auf; Text und Shape werden
// vor dem modalen Dialog kopiert und danach wird der Editor erneut abgefragt.
// Nachrichten: prompt_multiline und MessageBoxW können reentrant Nachrichten auslösen;
// über diese Aufrufe wird keine Referenz auf den Editorzustand gehalten.
pub(super) unsafe fn edit_at(h: H, point: ShapePoint, create: bool) -> bool {
    let hit = hit_callout(editor(h), point);
    if hit.is_none() && !create {
        return false;
    }
    let initial = hit
        .as_ref()
        .map(|(_, data)| data.text.replace('\n', "\r\n"));
    let Some(text) = text_dialog::prompt_multiline(h, initial.as_deref().unwrap_or("")) else {
        return true;
    };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.trim().is_empty() {
        return true;
    }
    let font_size = hit
        .as_ref()
        .map(|(_, data)| data.layout.font_size)
        .unwrap_or_else(|| editor(h).current_style().font_size);
    let layout = match shotkit::alpha_render::measure_callout_text(&text, font_size) {
        Ok(layout) => layout,
        Err(error) => {
            MessageBoxW(
                h,
                wide(&format!(
                    "Der Text konnte nicht vermessen werden: {error:?}"
                ))
                .as_ptr(),
                wide("Sprechblase").as_ptr(),
                0x10,
            );
            return true;
        }
    };
    if let Some((index, mut data)) = hit {
        data.text = text;
        data.layout = layout;
        apply_annotation_edit(h, |state| {
            state.replace_callout(index, data);
        });
    } else {
        let tip = Some(ShapePoint::new(
            point.x + 20.0,
            point.y + layout.body_height + 35.0,
        ));
        let data = CalloutData {
            origin: point,
            text,
            fill_color: Color::rgba(255, 255, 255, 255),
            text_color: Color::rgba(0, 0, 0, 255),
            tip,
            layout,
        };
        apply_annotation_edit(h, |state| {
            state.place_callout(data);
        });
    }
    report_editor_error(h);
    let (x, y, width, height) = image_view(h);
    let rect = Rect {
        left: x,
        top: y,
        right: x.saturating_add(width),
        bottom: y.saturating_add(height),
    };
    InvalidateRect(h, &rect, 0);
    true
}
