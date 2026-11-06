use super::*;
use crate::{
    image_edit::{FrameOptions, ShadowOptions},
    shapes::{Point, ShapeKind},
};

fn image() -> Image {
    Image::new(
        3,
        2,
        vec![
            0, 0, 0, 0, 1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 0,
        ],
    )
    .unwrap()
}
fn shape() -> Shape {
    Shape::new(ShapeKind::Line {
        start: Point::new(2.0, 3.0),
        end: Point::new(8.0, 9.0),
    })
}

#[test]
fn cursor_shape_move_remove_and_undo_preserve_image() {
    use crate::{cursor_snapshot::CursorPixels, shapes::CursorSprite};
    let original = image();
    let mut document = Document::new(original.clone()).unwrap();
    let sprite = CursorSprite::new(
        Point::new(0.0, 0.0),
        1,
        1,
        CursorPixels::Raster {
            and_mask: vec![255],
            xor_rgb: vec![255; 3],
        },
    )
    .unwrap();
    document
        .add_shape(Shape::new(ShapeKind::Cursor { data: sprite }))
        .unwrap();
    document
        .edit_shape(0, |shape| shape.offset(1.0, 1.0))
        .unwrap();
    assert_eq!(document.shapes()[0].bounds().x, 1.0);
    document.remove_shape(0).unwrap();
    assert!(document.shapes().is_empty());
    assert!(document.undo());
    assert_eq!(document.shapes()[0].bounds().x, 1.0);
    assert!(document.undo());
    assert_eq!(document.shapes()[0].bounds().x, 0.0);
    assert!(document.undo());
    assert!(document.shapes().is_empty());
    assert_eq!(document.image(), &original);
    assert!(document.redo());
    assert!(document.redo());
    assert_eq!(document.shapes()[0].bounds().x, 1.0);
}

#[test]
fn saved_revision_tracks_undo_redo_and_new_capture() {
    let mut d = Document::new(image()).unwrap();
    assert!(d.is_dirty()); // An unsaved capture stays dirty even at its initial state.
    d.add_shape(shape()).unwrap();
    assert!(d.undo());
    assert!(d.is_dirty());
    d.mark_saved();
    assert!(!d.is_dirty());
    assert!(d.redo());
    assert!(d.is_dirty());
    d.mark_saved();
    assert!(!d.is_dirty());
    assert!(d.undo());
    assert!(d.is_dirty());
    assert!(d.redo());
    assert!(!d.is_dirty());
}

#[test]
fn branching_after_undo_never_reuses_saved_revision() {
    let mut d = Document::new(image()).unwrap();
    d.mark_saved();
    d.add_shape(shape()).unwrap();
    d.mark_saved();
    assert!(d.undo());
    d.add_shape(shape()).unwrap(); // Same content and depth, but a new history branch.
    assert!(d.is_dirty());
    assert!(!d.redo());
}

#[test]
fn transactions_and_failed_edits_preserve_saved_revision() {
    let mut d = Document::new(image()).unwrap();
    d.mark_saved();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    assert!(d.is_dirty());
    assert!(!d.mark_saved());
    d.abort_edit().unwrap();
    assert!(!d.is_dirty());
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    assert!(d.commit_edit().unwrap());
    assert!(d.is_dirty());
    assert!(d.undo());
    assert!(!d.is_dirty());
    d.set_history_budget(0);
    assert!(d.add_shape(shape()).is_err());
    assert!(!d.is_dirty());
}

#[test]
fn peek_image_follows_undo_and_redo_order() {
    let mut d = Document::new(image()).unwrap();
    assert!(d.undo_image_arc().is_none());
    let original = d.image_arc();
    d.crop(CropRect {
        x: 1,
        y: 0,
        width: 2,
        height: 2,
    })
    .unwrap();
    let cropped = d.image_arc();
    assert!(Arc::ptr_eq(&original, &d.undo_image_arc().unwrap()));
    assert!(d.undo());
    assert!(Arc::ptr_eq(&cropped, &d.redo_image_arc().unwrap()));
}
#[test]
fn shared_history_orders_shape_and_image_changes() {
    let mut d = Document::new(image()).unwrap();
    d.add_shape(shape()).unwrap();
    assert_eq!(d.shapes().len(), 1);
    d.crop(CropRect {
        x: 1,
        y: 0,
        width: 2,
        height: 2,
    })
    .unwrap();
    assert_eq!((d.image().width, d.image().height), (2, 2));
    assert_eq!(d.shapes()[0].bounds().x, 1.0);
    assert!(d.undo()); // undo image crop and shape offset together
    assert_eq!((d.image().width, d.image().height), (3, 2));
    assert_eq!(d.shapes()[0].bounds().x, 2.0);
    assert!(d.undo());
    assert!(d.shapes().is_empty());
    assert!(d.redo());
    assert_eq!(d.shapes().len(), 1);
    assert!(d.redo());
    assert_eq!((d.image().width, d.image().height), (2, 2));
}

#[test]
fn frame_keeps_transparent_source_pixel_and_offsets_shapes() {
    let mut d = Document::new(image()).unwrap();
    d.add_shape(shape()).unwrap();
    d.add_frame(FrameOptions {
        width: 1,
        color: crate::shapes::Color::rgba(255, 0, 0, 255),
        shadow: Some(ShadowOptions {
            offset_x: 2,
            offset_y: 0,
            blur_radius: 1,
            opacity: 80,
        }),
    })
    .unwrap();
    assert_eq!(d.shapes()[0].bounds().x, 4.0); // margin left is 1 plus frame width
    let content_x = 1 + 1;
    let content_y = 1 + 1;
    let offset = (content_y * d.image().width + content_x) * 4;
    assert_eq!(&d.image().pixels[offset..offset + 4], &image().pixels[..4]);
    assert!(d.undo());
    assert_eq!(d.shapes()[0].bounds().x, 2.0);
    assert_eq!((d.image().width, d.image().height), (3, 2));
}

#[test]
fn transaction_abort_restores_image_and_shapes_and_adds_no_history() {
    let mut d = Document::new(image()).unwrap();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    d.crop(CropRect {
        x: 1,
        y: 0,
        width: 2,
        height: 2,
    })
    .unwrap();
    assert_eq!(d.shapes().len(), 1);
    assert_eq!(d.image().width, 2);
    d.abort_edit().unwrap();
    assert!(d.shapes().is_empty());
    assert_eq!((d.image().width, d.image().height), (3, 2));
    assert!(!d.can_undo());
    assert!(!d.undo());
}

#[test]
fn budget_is_enforced_and_oldest_snapshots_are_evicted() {
    let mut d = Document::with_history_budget(image(), 8192).unwrap();
    for i in 0..100 {
        d.add_shape(Shape::new(ShapeKind::Step {
            center: Point::new(i as f32, 0.0),
            number: i,
        }))
        .unwrap();
    }
    assert!(d.history_bytes() <= d.history_budget());
    let mut count = 0;
    while d.undo() {
        count += 1;
    }
    assert!(count > 0 && count < 100);
    let mut tight = Document::with_history_budget(image(), 1).unwrap();
    assert_eq!(
        tight.add_shape(shape()),
        Err(DocumentError::HistoryBudgetExceeded)
    );
    assert!(tight.shapes().is_empty());
    assert!(!tight.can_undo());
}

#[test]
fn large_current_image_is_not_charged_again_for_shape_history() {
    let large = Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap();
    let mut d = Document::with_history_budget(large, 512).unwrap();
    d.add_shape(shape()).unwrap();
    assert!(d.history_bytes() <= d.history_budget());
    assert!(d.undo());
    assert!(d.shapes().is_empty());
}

#[test]
fn crop_rejects_snapshot_that_exceeds_budget_and_preserves_redo() {
    let large = Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap();
    let mut d = Document::with_history_budget(large.clone(), 4096).unwrap();
    d.add_shape(shape()).unwrap();
    assert!(d.undo());
    assert!(d.can_redo());
    assert_eq!(
        d.crop(CropRect {
            x: 0,
            y: 0,
            width: 64,
            height: 64
        }),
        Err(DocumentError::HistoryBudgetExceeded)
    );
    assert_eq!(d.image(), &large);
    assert!(d.can_redo());
    assert!(d.history_bytes() <= d.history_budget());
    assert!(d.redo());
}

#[test]
fn image_change_trims_history_using_new_current_image() {
    let large = Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap();
    let mut d = Document::with_history_budget(large, 70_000).unwrap();
    for _ in 0..20 {
        d.add_shape(shape()).unwrap();
    }
    d.crop(CropRect {
        x: 0,
        y: 0,
        width: 64,
        height: 64,
    })
    .unwrap();
    assert!(d.history_bytes() <= d.history_budget());
    assert!(d.can_undo());
    assert!(d.undo());
    assert_eq!(d.image().width, 128);
}

#[test]
fn transaction_image_change_rejects_unaffordable_snapshot() {
    let large = Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap();
    let mut d = Document::with_history_budget(large.clone(), 512).unwrap();
    d.begin_edit().unwrap();
    d.crop(CropRect {
        x: 0,
        y: 0,
        width: 64,
        height: 64,
    })
    .unwrap();
    assert_eq!(d.commit_edit(), Err(DocumentError::HistoryBudgetExceeded));
    d.abort_edit().unwrap();
    assert_eq!(d.image(), &large);
    assert!(d.history_bytes() <= d.history_budget());
}

#[test]
fn undo_evicts_redo_state_that_cannot_fit_budget() {
    let mut d = Document::with_history_budget(image(), 1024).unwrap();
    d.replace_image(Image::new(16, 16, vec![9; 16 * 16 * 4]).unwrap())
        .unwrap();
    assert!(d.undo());
    assert_eq!((d.image().width, d.image().height), (3, 2));
    assert!(!d.can_redo());
    assert!(d.history_bytes() <= d.history_budget());
}

#[test]
fn redo_is_preserved_when_result_fits_and_stays_within_budget() {
    let mut d = Document::with_history_budget(image(), 4096).unwrap();
    let replacement = Image::new(4, 4, vec![11; 4 * 4 * 4]).unwrap();
    d.replace_image(replacement.clone()).unwrap();
    assert!(d.undo());
    assert!(d.can_redo());
    assert!(d.history_bytes() <= d.history_budget());
    assert!(d.redo());
    assert_eq!(d.image(), &replacement);
    assert!(d.history_bytes() <= d.history_budget());
}

#[test]
fn tiny_image_history_counts_snapshot_and_allocation_overhead() {
    let mut d = Document::with_history_budget(image(), 4096).unwrap();
    for value in 0..100u8 {
        d.replace_image(Image::new(1, 1, vec![value; 4]).unwrap())
            .unwrap();
        assert!(d.history_bytes() <= d.history_budget());
    }
    assert!(d.undo.len() < 100);
    assert!(d.history_bytes() >= d.undo.len() * size_of::<State>());
    assert!(d.history_bytes() > d.undo.len() * 4);
}

#[test]
fn invalid_images_are_rejected_without_replacing_current_state() {
    let invalid = Image {
        width: 2,
        height: 2,
        pixels: vec![1, 2, 3, 4],
    };
    assert!(matches!(
        Document::new(invalid.clone()),
        Err(DocumentError::InvalidImage)
    ));

    let mut d = Document::new(image()).unwrap();
    assert_eq!(d.replace_image(invalid), Err(DocumentError::InvalidImage));
    assert_eq!((d.image().width, d.image().height), (3, 2));
    assert_eq!(d.image().pixels, image().pixels);
    assert!(!d.can_undo());
}

#[test]
fn recover_constructor_returns_invalid_image_unchanged() {
    let invalid = Image {
        width: 2,
        height: 2,
        pixels: vec![1, 2, 3, 4],
    };
    let (error, recovered) = Document::new_recover(invalid.clone()).unwrap_err();
    assert_eq!(error, DocumentError::InvalidImage);
    assert_eq!(recovered, invalid);

    let valid = image();
    assert_eq!(
        Document::new_recover(valid.clone()).unwrap().image(),
        &valid
    );
}

#[test]
fn failed_transaction_commit_keeps_state_open_until_abort() {
    let mut d = Document::with_history_budget(image(), 512).unwrap();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    d.set_history_budget(1);

    assert_eq!(d.commit_edit(), Err(DocumentError::HistoryBudgetExceeded));
    assert_eq!(d.shapes().len(), 1);
    assert_eq!(d.begin_edit(), Err(DocumentError::TransactionAlreadyOpen));
    d.abort_edit().unwrap();
    assert!(d.shapes().is_empty());
    assert_eq!(d.image().pixels, image().pixels);
    assert!(!d.can_undo());
}

#[test]
fn canvas_expansion_preserves_bgra_and_moves_shapes_with_undo_redo() {
    let mut d = Document::new(image()).unwrap();
    d.add_shape(shape()).unwrap();
    let old_image = d.image().clone();
    let old_shapes = d.shapes().to_vec();
    d.resize_canvas(
        6,
        4,
        CanvasAlignment::Center,
        CanvasAlignment::End,
        Color::rgba(10, 20, 30, 40),
    )
    .unwrap();
    assert_eq!(&d.image().pixels[..4], &[30, 20, 10, 40]);
    let offset = (2 * 6 + 1) * 4;
    assert_eq!(
        &d.image().pixels[offset..offset + 12],
        &old_image.pixels[..12]
    );
    assert_eq!(d.shapes()[0].bounds().x, old_shapes[0].bounds().x + 1.0);
    assert_eq!(d.shapes()[0].bounds().y, old_shapes[0].bounds().y + 2.0);
    let expanded = d.image().clone();
    assert!(d.undo());
    assert_eq!(d.image(), &old_image);
    assert_eq!(d.shapes(), old_shapes);
    assert!(d.redo());
    assert_eq!(d.image(), &expanded);
}

#[test]
fn canvas_shrink_clips_pixels_but_keeps_editable_shapes() {
    let mut d = Document::new(image()).unwrap();
    d.add_shape(shape()).unwrap();
    d.resize_canvas(
        2,
        1,
        CanvasAlignment::End,
        CanvasAlignment::End,
        Color::rgba(0, 0, 0, 0),
    )
    .unwrap();
    assert_eq!(d.image().pixels, image().pixels[16..24]);
    assert_eq!(d.shapes()[0].bounds().x, 1.0);
    assert_eq!(d.shapes()[0].bounds().y, 2.0);
    assert!(d.undo());
    assert_eq!(d.image(), &image());
}

#[test]
fn canvas_limits_and_history_failure_are_atomic() {
    let mut d = Document::new(image()).unwrap();
    for (w, h) in [(0, 1), (30_001, 1), (30_000, 30_000), (usize::MAX, 2)] {
        assert_eq!(
            d.resize_canvas(
                w,
                h,
                CanvasAlignment::Start,
                CanvasAlignment::Start,
                Color::RED
            ),
            Err(DocumentError::InvalidImage)
        );
        assert_eq!(d.image(), &image());
        assert!(!d.can_undo());
    }
    d.set_history_budget(0);
    assert_eq!(
        d.resize_canvas(
            4,
            4,
            CanvasAlignment::Start,
            CanvasAlignment::Start,
            Color::RED
        ),
        Err(DocumentError::HistoryBudgetExceeded)
    );
    assert_eq!(d.image(), &image());
    assert!(!d.can_undo());
}
