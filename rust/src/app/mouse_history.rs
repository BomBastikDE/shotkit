use super::*;
use shotkit::mouse_history::{since_previous, MouseSample};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NativeMouseSample {
    x: i32,
    y: i32,
    time: u32,
    extra_info: usize,
}

#[link(name = "user32")]
extern "system" {
    fn ClientToScreen(hwnd: H, point: *mut Point) -> i32;
    fn ScreenToClient(hwnd: H, point: *mut Point) -> i32;
    fn GetMessageTime() -> i32;
    fn GetMouseMovePointsEx(
        size: u32,
        input: *const NativeMouseSample,
        output: *mut NativeMouseSample,
        count: i32,
        resolution: u32,
    ) -> i32;
}

/// Recover the bounded movement history only while a known prior sample is
/// still present. Missing or ambiguous history falls back to the current event.
// SAFETY CONTRACT:
// Voraussetzung: h ist ein gültiges Editorfenster auf dem UI-Thread; l enthält
// die Client-Koordinaten der aktuell verarbeiteten WM_MOUSEMOVE-Nachricht.
// Warum erfüllt: Der Maus-Handler ruft die Funktion für sein eigenes Fenster
// auf; alle Windows-Puffer sind lokale, korrekt dimensionierte Werte und kein
// Fensterzustands-Borrow bleibt über einen API-Aufruf bestehen.
// Nachrichten: Die aufgerufenen Koordinaten- und Historienfunktionen versenden
// keine expliziten Fensternachrichten; Fehlschläge fallen auf das Ereignis zurück.
pub(super) unsafe fn pen_samples(h: H, l: L) -> (Vec<ShapePoint>, Option<MouseSample>) {
    let fallback = || {
        let (x, y) = point(h, l);
        (vec![ShapePoint::new(x as f32, y as f32)], None)
    };
    let mut screen = Point {
        x: (l as u32 as u16 as i16) as i32,
        y: ((l as u32 >> 16) as u16 as i16) as i32,
    };
    if ClientToScreen(h, &mut screen) == 0 {
        return fallback();
    }
    let previous = window_state(h).pen_history;
    let input = NativeMouseSample {
        x: screen.x & 0xffff,
        y: screen.y & 0xffff,
        time: GetMessageTime() as u32,
        extra_info: 0,
    };
    let mut native = [NativeMouseSample::default(); 64];
    let count = GetMouseMovePointsEx(
        std::mem::size_of::<NativeMouseSample>() as u32,
        &input,
        native.as_mut_ptr(),
        native.len() as i32,
        1, // GMMP_USE_DISPLAY_POINTS
    );
    if count <= 0 || count as usize > native.len() {
        return fallback();
    }
    let history: Vec<_> = native[..count as usize]
        .iter()
        .map(|p| MouseSample {
            x: (p.x as u16 as i16) as i32,
            y: (p.y as u16 as i16) as i32,
            time: p.time,
        })
        .collect();
    let current = history[0];
    if current.x != screen.x || current.y != screen.y {
        return fallback();
    }
    let selected = since_previous(&history, previous);
    let mut client = Vec::with_capacity(selected.len());
    for sample in selected {
        let mut p = Point {
            x: sample.x,
            y: sample.y,
        };
        if ScreenToClient(h, &mut p) == 0 {
            return fallback();
        }
        client.push(p);
    }
    let (x, y, width, height) = image_view(h);
    let v = view::View {
        x,
        y,
        width,
        height,
    };
    let image = (
        window_state(h).width.max(1) as u32,
        window_state(h).height.max(1) as u32,
    );
    let positions = client
        .into_iter()
        .map(|p| {
            let (x, y) = view::to_image(v, image, (p.x, p.y));
            ShapePoint::new(x as f32, y as f32)
        })
        .collect();
    (positions, Some(current))
}
