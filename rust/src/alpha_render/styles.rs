//! Non-default stroke and endpoint rendering; legacy geometry stays in the parent.
use super::*;
use crate::shapes::{
    arrow_dot_radius, rounded_rectangle_points, styled_arrow_bounds, styled_arrow_head, ArrowEnds,
    LineStyle, Point,
};

// SAFETY CONTRACT:
// Voraussetzung: pen ist ein gültiger exklusiv verwendeter GDI+-Pen einer aktiven Session.
// Warum erfüllt: draw_shape besitzt den geprüften Pen-Guard und überlebt diesen Aufruf;
// nur definierte GDI+-DashStyle-Werte werden übergeben, der Status wird geprüft.
// Nachrichten: GDI+-Aufruf ohne übergebene Callback-Pointer; kein Eigentumswechsel.
pub(super) unsafe fn set_line_style(pen: *mut c_void, style: LineStyle) -> Result<(), RenderError> {
    let dash_style = match style {
        LineStyle::Solid => return Ok(()),
        LineStyle::Dashed => 1,
        LineStyle::Dotted => 2,
    };
    status(GdipSetPenDashStyle(pen, dash_style))
}

// SAFETY CONTRACT:
// Voraussetzung: g und pen sind lebende Ressourcen derselben aktiven GDI+-Session.
// Warum erfüllt: draw_shape hält Graphics und Pen bis zur Rückkehr; PointF-Arrays
// und lokale Brush-Guards leben über alle FFI-Aufrufe, deren Status geprüft wird.
// Nachrichten: GDI+-Aufrufe; keine Pointer oder Ressourcen werden extern gespeichert.
pub(super) unsafe fn draw_arrow(
    g: *mut c_void,
    pen: *mut c_void,
    shape: &Shape,
) -> Result<(), RenderError> {
    let ShapeKind::Arrow { start, end } = shape.kind else {
        return Ok(());
    };
    let Some(head) = styled_arrow_head(start, end, shape.style.width) else {
        return Ok(());
    };
    if styled_arrow_bounds(start, end, &shape.style).is_none() {
        return Ok(());
    }
    let brush = create_brush(argb(shape.style.color, 255))?;
    if shape.style.arrow_ends == ArrowEnds::Dot {
        status(GdipDrawLine(g, pen, start.x, start.y, end.x, end.y))?;
        let radius = arrow_dot_radius(shape.style.width);
        return status(GdipFillEllipse(
            g,
            brush.as_ptr(),
            end.x - radius,
            end.y - radius,
            radius * 2.0,
            radius * 2.0,
        ));
    }
    let Some(back) = styled_arrow_head(end, start, shape.style.width) else {
        return Ok(());
    };
    if let (Some(a), Some(b)) = (back.shaft_end, head.shaft_end) {
        status(GdipDrawLine(g, pen, a.x, a.y, b.x, b.y))?;
    }
    for head in [head, back] {
        let points = [head.tip, head.left, head.right].map(|p| PointF { x: p.x, y: p.y });
        status(GdipFillPolygon(g, brush.as_ptr(), points.as_ptr(), 3, 0))?;
    }
    Ok(())
}

// SAFETY CONTRACT:
// Voraussetzung: g und pen sind lebende Ressourcen einer aktiven GDI+-Session.
// Warum erfüllt: draw_shape hält beide Ressourcen; das feste geschlossene Array
// und optionaler Brush leben bis nach ihren FFI-Aufrufen; Status wird geprüft.
// Nachrichten: GDI+-Aufrufe; kein Eigentumswechsel und keine Callbacks.
pub(super) unsafe fn draw_rounded_rectangle(
    g: *mut c_void,
    pen: *mut c_void,
    shape: &Shape,
    start: Point,
    end: Point,
) -> Result<(), RenderError> {
    let Some(points) = rounded_rectangle_points(start, end) else {
        return Ok(());
    };
    let points = points.map(|p| PointF { x: p.x, y: p.y });
    if let Some(alpha) = shape.style.fill.alpha_scale() {
        let brush = create_brush(argb(shape.style.color, alpha))?;
        status(GdipFillPolygon(g, brush.as_ptr(), points.as_ptr(), 36, 0))?;
    }
    status(GdipDrawLines(g, pen, points.as_ptr(), points.len() as i32))
}
