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
    assert!(original.ptr_eq(&d.undo_image_arc().unwrap()));
    assert!(d.undo());
    assert!(cropped.ptr_eq(&d.redo_image_arc().unwrap()));
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
    // Enough for the current 100-shape snapshot and its inverse, but far too
    // small for all historical snapshots: oldest eviction must still occur.
    let mut d = Document::with_history_budget(image(), 64 * 1024).unwrap();
    for i in 0..100 {
        d.add_shape(Shape::new(ShapeKind::Step {
            center: Point::new(i as f32, 0.0),
            number: i,
        }))
        .unwrap();
    }
    assert!(d.history_bytes() <= d.history_budget());
    let retained = d.undo.len();
    assert!(retained > 0 && retained < 100);
    // Give navigation enough room to retain all remaining steps. The edit
    // phase above still proves older snapshots were evicted at 64 KiB.
    d.set_history_budget(4 * 1024 * 1024);
    let mut count = 0;
    while d.undo() {
        count += 1;
    }
    assert_eq!(count, retained);
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
    // Far below the 64-KiB image allocation, while allowing shape metadata
    // for both the snapshot and its immediate inverse.
    let mut d = Document::with_history_budget(large, 4096).unwrap();
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
fn undo_rejects_when_it_would_evict_redo_and_preserves_document_state() {
    let mut d = Document::with_history_budget(image(), 1024).unwrap();
    let original = d.image().clone();
    d.mark_saved();
    let replacement = Image::new(16, 16, vec![9; 16 * 16 * 4]).unwrap();
    d.replace_image(replacement.clone()).unwrap();
    let bytes_before = d.history_bytes();

    assert_eq!(
        d.check_undo(),
        Err(DocumentError::HistoryNavigationBudgetExceeded)
    );
    assert_eq!(
        d.try_undo(),
        Err(DocumentError::HistoryNavigationBudgetExceeded)
    );
    assert_eq!(d.image(), &replacement);
    assert!(d.can_undo());
    assert!(!d.can_redo());
    assert!(d.is_dirty());
    assert_eq!(d.history_bytes(), bytes_before);
    assert_eq!(
        DocumentError::HistoryNavigationBudgetExceeded.to_string(),
        "Diese Aktion wurde nicht ausgeführt, weil das Speicherlimit für die Rückgängig-Historie nicht ausreicht. Bild und Verlauf bleiben unverändert."
    );

    d.set_history_budget(8 * 1024);
    assert!(d.try_undo().unwrap());
    assert_eq!(d.image(), &original);
    assert!(!d.can_undo());
    assert!(d.can_redo());
    assert!(!d.is_dirty());
    assert!(d.try_redo().unwrap());
    assert_eq!(d.image(), &replacement);
    assert!(d.can_undo());
    assert!(!d.can_redo());
    assert!(d.is_dirty());
}

#[test]
fn undo_at_exact_minimum_budget_keeps_the_immediate_redo() {
    let mut d = Document::with_history_budget(image(), 8192).unwrap();
    let original = d.image().clone();
    let replacement = Image::new(16, 16, vec![9; 16 * 16 * 4]).unwrap();
    d.replace_image(replacement.clone()).unwrap();
    let mut planned_redo: Vec<State> = Vec::new();
    planned_redo.try_reserve_exact(1).unwrap();
    let minimum = size_of::<State>()
        + revision_allocation_bytes()
        + image_allocation_bytes(&d.current.image)
        + shapes_allocation_bytes(&d.current.shapes)
        + container_bytes(planned_redo.capacity(), size_of::<State>());
    d.set_history_budget(minimum - 1);
    let bytes_before = d.history_bytes();
    assert_eq!(
        d.try_undo(),
        Err(DocumentError::HistoryNavigationBudgetExceeded)
    );
    assert_eq!(d.image(), &replacement);
    assert!(d.can_undo());
    assert_eq!(d.history_bytes(), bytes_before);
    d.set_history_budget(minimum);
    assert_eq!(d.check_undo(), Ok(true));
    assert!(d.try_undo().unwrap());
    assert_eq!(d.image(), &original);
    assert!(d.can_redo());
    assert!(d.history_bytes() <= d.history_budget());
    assert!(d.try_redo().unwrap());
    assert_eq!(d.image(), &replacement);
}

#[test]
fn history_navigation_checks_are_false_without_entries_or_during_staging() {
    let mut d = Document::new(image()).unwrap();
    assert_eq!(d.check_undo(), Ok(false));
    assert_eq!(d.check_redo(), Ok(false));
    assert_eq!(d.try_undo(), Ok(false));
    assert_eq!(d.try_redo(), Ok(false));

    let replacement = Image::new(4, 4, vec![11; 4 * 4 * 4]).unwrap();
    d.replace_image(replacement.clone()).unwrap();
    assert!(d.try_undo().unwrap());
    assert_eq!(d.image(), &image());
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let staged_image = d.image().clone();
    let staged_shapes = d.shapes().to_vec();
    let bytes_before = d.history_bytes();

    assert_eq!(d.check_undo(), Ok(false));
    assert_eq!(d.check_redo(), Ok(false));
    assert_eq!(d.try_undo(), Ok(false));
    assert_eq!(d.try_redo(), Ok(false));
    assert_eq!(d.image(), &staged_image);
    assert_eq!(d.shapes(), staged_shapes);
    assert!(!d.can_redo());
    assert_eq!(d.history_bytes(), bytes_before);

    d.abort_edit().unwrap();
    assert_eq!(d.image(), &image());
    assert!(d.can_redo());
    assert!(d.try_redo().unwrap());
    assert_eq!(d.image(), &replacement);
}

#[test]
fn redo_preserves_all_history_when_states_compete_for_budget() {
    let mut d = Document::with_history_budget(image(), 80 * 1024).unwrap();
    let large = Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap();
    let small_next = Image::new(1, 1, vec![11; 4]).unwrap();
    let small_later = Image::new(1, 1, vec![22; 4]).unwrap();
    d.replace_image(large.clone()).unwrap();
    d.replace_image(small_next.clone()).unwrap();
    d.replace_image(small_later).unwrap();
    assert!(d.try_undo().unwrap());
    assert!(d.try_undo().unwrap());
    assert_eq!(d.image(), &large);
    let target = d.redo_image_arc().unwrap();
    let minimum = d.snapshot_bytes_after_redo_clear(&d.current, &target);
    d.set_history_budget(minimum);
    let undo_count = d.undo.len();
    let redo_count = d.redo.len();
    let undo_before: Vec<_> = d.undo.iter().cloned().collect();
    let redo_before = d.redo.to_vec();
    let bytes_before = d.history_bytes();
    assert_eq!(
        d.check_redo(),
        Err(DocumentError::HistoryNavigationBudgetExceeded)
    );
    assert_eq!(
        d.try_redo(),
        Err(DocumentError::HistoryNavigationBudgetExceeded)
    );
    assert_eq!(d.image(), &large);
    assert_eq!(d.undo.len(), undo_count);
    assert_eq!(d.redo.len(), redo_count);
    assert!(d.undo.iter().eq(undo_before.iter()));
    assert!(d.redo.iter().eq(redo_before.iter()));
    assert_eq!(d.history_bytes(), bytes_before);
    d.set_history_budget(80 * 1024);
    assert!(d.try_redo().unwrap());
    assert_eq!(d.image(), &small_next);
    assert_eq!(d.redo.len(), redo_count - 1);
    assert!(d.can_undo());
    assert!(d.history_bytes() <= d.history_budget());
    assert!(d.try_undo().unwrap());
    assert_eq!(d.image(), &large);
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

#[test]
fn canvas_budget_rejection_is_atomic_and_keeps_existing_reservations() {
    use crate::resource_budget::{AdmissionClass, ResourceBudget};

    let mut document = Document::new(image()).unwrap();
    document.add_shape(shape()).unwrap();
    assert!(document.mark_saved());
    document.add_shape(shape()).unwrap();
    assert!(document.undo());
    let before = document.image().clone();
    // The 32-byte output fits the 39-byte ceiling alone; the existing 8-byte
    // reservation is what makes admission fail (40 bytes total).
    let budget = ResourceBudget::new(39, 0).unwrap();
    let blocker = budget.try_acquire(8, AdmissionClass::Reserved).unwrap();

    assert_eq!(
        document.resize_canvas_with_budget(
            4,
            2,
            CanvasAlignment::Start,
            CanvasAlignment::Start,
            Color::RED,
            &budget,
        ),
        Err(DocumentError::ImageBudgetExceeded)
    );
    assert_eq!(document.image(), &before);
    assert_eq!(document.shapes().len(), 1);
    assert!(!document.is_dirty());
    assert!(document.can_undo());
    assert!(document.can_redo());
    assert_eq!(budget.snapshot().used_bytes, 8);

    drop(blocker);
    assert_eq!(budget.snapshot().used_bytes, 0);
}

#[test]
fn canvas_budget_lease_follows_history_and_releases_on_staged_abort() {
    use crate::resource_budget::ResourceBudget;

    let mut document = Document::new(image()).unwrap();
    let budget = ResourceBudget::new(256, 0).unwrap();
    document
        .resize_canvas_with_budget(
            4,
            3,
            CanvasAlignment::Center,
            CanvasAlignment::End,
            Color::rgba(1, 2, 3, 4),
            &budget,
        )
        .unwrap();
    let first_allocation = document.image().pixels.capacity();
    assert!(first_allocation >= 48);
    assert_eq!(budget.snapshot().used_bytes, first_allocation);

    assert!(document.undo());
    assert_eq!(budget.snapshot().used_bytes, first_allocation);
    assert!(document.redo());
    assert_eq!(budget.snapshot().used_bytes, first_allocation);

    let before = document.image().clone();
    document.begin_edit().unwrap();
    document
        .resize_canvas_with_budget(
            5,
            4,
            CanvasAlignment::Start,
            CanvasAlignment::Start,
            Color::rgba(0, 0, 0, 0),
            &budget,
        )
        .unwrap();
    let second_allocation = document.image().pixels.capacity();
    assert!(second_allocation >= 80);
    assert_eq!(
        budget.snapshot().used_bytes,
        first_allocation + second_allocation
    );
    document.abort_edit().unwrap();
    assert_eq!(document.image(), &before);
    assert_eq!(budget.snapshot().used_bytes, first_allocation);
    drop(document);
    assert_eq!(budget.snapshot().used_bytes, 0);
}

#[test]
fn budgeted_canvas_resize_matches_legacy_pixels_and_shapes() {
    use crate::resource_budget::ResourceBudget;

    let mut legacy = Document::new(image()).unwrap();
    let mut budgeted = Document::new(image()).unwrap();
    legacy.add_shape(shape()).unwrap();
    budgeted.add_shape(shape()).unwrap();
    let budget = ResourceBudget::new(256, 0).unwrap();

    legacy
        .resize_canvas(
            5,
            4,
            CanvasAlignment::Center,
            CanvasAlignment::End,
            Color::rgba(9, 17, 31, 73),
        )
        .unwrap();
    budgeted
        .resize_canvas_with_budget(
            5,
            4,
            CanvasAlignment::Center,
            CanvasAlignment::End,
            Color::rgba(9, 17, 31, 73),
            &budget,
        )
        .unwrap();

    assert_eq!(budgeted.image(), legacy.image());
    assert_eq!(budgeted.shapes(), legacy.shapes());
    assert_eq!(
        budget.snapshot().used_bytes,
        budgeted.image().pixels.capacity()
    );
}

// Include identities and allocation capacities as well as logical contents:
// rejected plans must preserve staged rollback, save state and both branches.
fn plan_observation(d: &Document) -> (String, Vec<usize>) {
    let mut identities = vec![
        Arc::as_ptr(&d.history_generation) as usize,
        d.undo.capacity(),
        d.redo.capacity(),
        usize::from(d.is_dirty()),
    ];
    identities.push(
        d.saved_revision
            .as_ref()
            .map_or(0, |r| Arc::as_ptr(r) as usize),
    );
    for state in std::iter::once(&d.current)
        .chain(d.staged_before.iter())
        .chain(d.undo.iter())
        .chain(d.redo.iter())
    {
        identities.extend([
            state.image.identity(),
            Arc::as_ptr(&state.revision) as usize,
            state.image.pixels.capacity(),
            shapes_allocation_bytes(&state.shapes),
        ]);
    }
    (format!("{d:?}"), identities)
}

#[test]
fn snapshot_planning_requires_transaction_and_leaves_no_op_untouched() {
    let mut d = Document::new(image()).unwrap();
    let before = plan_observation(&d);
    assert!(matches!(
        d.plan_commit_edit(),
        Err(DocumentError::NoTransaction)
    ));
    assert_eq!(plan_observation(&d), before);
    d.begin_edit().unwrap();
    let before = plan_observation(&d);
    assert!(d.plan_commit_edit().unwrap().is_none());
    assert_eq!(plan_observation(&d), before);
    assert!(!d.commit_edit().unwrap());
}

#[test]
fn prepared_snapshot_accounts_actual_capacities_and_preserves_shared_images() {
    let large = Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap();
    let mut d = Document::with_history_budget(large, 8192).unwrap();
    d.add_shape(shape()).unwrap();
    d.add_shape(shape()).unwrap();
    d.undo.try_reserve_exact(128).unwrap(); // Excess capacity need not be retained.
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let before = plan_observation(&d);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    assert_eq!(plan_observation(&d), before);
    assert_eq!(plan.undo_entries_to_evict(), 0);
    assert_eq!(plan.redo_entries_to_discard(), 0);
    let expected = d.undo.iter().map(snapshot_storage_bytes).sum::<usize>()
        + snapshot_storage_bytes(plan.snapshot.as_ref().unwrap())
        + container_bytes(plan.undo.capacity(), size_of::<State>());
    assert_eq!(plan.history_bytes(), expected); // No duplicate 64 KiB image.
    assert_eq!(plan.history_budget(), d.history_budget());
    let planned_bytes = plan.history_bytes();
    d.commit_edit_with_plan(plan, false).unwrap();
    assert_eq!(d.history_bytes(), planned_bytes);
    assert!(d.undo());
    assert_eq!(d.shapes().len(), 2);
}

#[test]
fn prepared_snapshot_reports_redo_reset_separately_from_undo_eviction() {
    let mut d = Document::new(image()).unwrap();
    d.add_shape(shape()).unwrap();
    d.add_shape(shape()).unwrap();
    assert!(d.undo());
    d.mark_saved();
    d.begin_edit().unwrap();
    d.add_shape(Shape::new(ShapeKind::Step {
        center: Point::new(1.0, 1.0),
        number: 1,
    }))
    .unwrap();
    let before = plan_observation(&d);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    assert_eq!(plan_observation(&d), before);
    assert_eq!(plan.undo_entries_to_evict(), 0);
    assert_eq!(plan.redo_entries_to_discard(), 1);
    assert_eq!(
        d.commit_edit_with_plan(plan, false),
        Err(DocumentError::HistoryEvictionNotApproved)
    );
    assert_eq!(plan_observation(&d), before);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    d.commit_edit_with_plan(plan, true).unwrap();
    assert!(!d.can_redo());
    assert!(d.is_dirty());
    assert!(d.undo());
    assert!(!d.is_dirty());
}

#[test]
fn undo_eviction_requires_explicit_approval_and_consumes_exact_plan() {
    let mut d = Document::new(image()).unwrap();
    for _ in 0..12 {
        d.add_shape(shape()).unwrap();
    }
    let old_count = d.undo.len();
    d.set_history_budget(d.history_bytes());
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let before = plan_observation(&d);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    let evictions = plan.undo_entries_to_evict();
    assert!(evictions > 0 && evictions < old_count);
    let mut one_more: VecDeque<State> = VecDeque::new();
    one_more
        .try_reserve_exact(old_count + 2 - evictions)
        .unwrap();
    let one_more_bytes = d
        .undo
        .iter()
        .skip(evictions - 1)
        .map(snapshot_storage_bytes)
        .sum::<usize>()
        + snapshot_storage_bytes(plan.snapshot.as_ref().unwrap())
        + container_bytes(one_more.capacity(), size_of::<State>());
    let one_more_inverse = NavigationPlan::reserve(old_count + 1 - evictions, 1).unwrap();
    let one_more_inverse_bytes = d
        .undo
        .iter()
        .skip(evictions - 1)
        .map(snapshot_storage_bytes)
        .sum::<usize>()
        + snapshot_storage_bytes(&d.current)
        + one_more_inverse.container_bytes();
    assert!(one_more_bytes.max(one_more_inverse_bytes) > d.history_budget());
    assert_eq!(plan_observation(&d), before);
    assert_eq!(
        d.commit_edit_with_plan(plan, false),
        Err(DocumentError::HistoryEvictionNotApproved)
    );
    assert_eq!(plan_observation(&d), before);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    assert_eq!(plan.undo_entries_to_evict(), evictions);
    let planned_bytes = plan.history_bytes();
    let expected_oldest_revision = Arc::clone(&d.undo[evictions].revision);
    d.commit_edit_with_plan(plan, true).unwrap();
    assert_eq!(d.undo.len(), old_count + 1 - evictions);
    assert!(Arc::ptr_eq(
        &d.undo.front().unwrap().revision,
        &expected_oldest_revision
    ));
    assert_eq!(d.history_bytes(), planned_bytes);
    assert!(planned_bytes <= d.history_budget());
    assert!(d.staged_before.is_none());
    assert!(d.try_undo().unwrap());
    assert_eq!(d.shapes().len(), 12);
    assert!(d.try_redo().unwrap());
    assert_eq!(d.shapes().len(), 13);
}

#[test]
fn shared_old_image_is_charged_once_across_retained_snapshots() {
    let mut pixels = Vec::with_capacity(4096);
    pixels.extend_from_slice(&image().pixels);
    let original = Image::new(3, 2, pixels).unwrap();
    let mut d = Document::new(original).unwrap();
    d.add_shape(shape()).unwrap();
    d.add_shape(shape()).unwrap();
    d.begin_edit().unwrap();
    d.replace_image(Image::new(1, 1, vec![99; 4]).unwrap())
        .unwrap();
    let plan = d.plan_commit_edit().unwrap().unwrap();
    let expected = d.undo.iter().map(snapshot_storage_bytes).sum::<usize>()
        + snapshot_storage_bytes(plan.snapshot.as_ref().unwrap())
        + image_allocation_bytes(&plan.snapshot.as_ref().unwrap().image)
        + container_bytes(plan.undo.capacity(), size_of::<State>());
    assert_eq!(plan.history_bytes(), expected);
    assert!(image_allocation_bytes(&plan.snapshot.as_ref().unwrap().image) > 4096);
    d.commit_edit_with_plan(plan, false).unwrap();
    assert_eq!(d.history_bytes(), expected);
}

#[test]
fn unaffordable_snapshot_plan_preserves_transaction_redo_and_history() {
    let mut d = Document::new(Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap()).unwrap();
    d.add_shape(shape()).unwrap();
    d.add_shape(shape()).unwrap();
    assert!(d.undo());
    d.set_history_budget(8192);
    d.begin_edit().unwrap();
    d.replace_image(image()).unwrap();
    let before = plan_observation(&d);
    assert!(matches!(
        d.plan_commit_edit(),
        Err(DocumentError::HistoryBudgetExceeded)
    ));
    assert_eq!(plan_observation(&d), before);
    assert!(!d.redo.is_empty());
    d.abort_edit().unwrap();
    assert_eq!(d.image().width, 128);
    assert!(d.can_redo());
}

#[test]
fn stale_snapshot_plans_reject_every_successful_mutation_without_side_effects() {
    let mutations: &[fn(&mut Document)] = &[
        |d| {
            d.add_shape(shape()).unwrap();
        }, // Staging retains the revision Arc.
        |d| {
            d.edit_shape(0, |s| s.offset(1.0, 0.0)).unwrap();
        },
        |d| {
            d.remove_shape(0).unwrap();
        },
        |d| {
            d.replace_image(Image::new(1, 1, vec![5; 4]).unwrap())
                .unwrap();
        },
        |d| {
            d.set_history_budget(d.history_budget());
        },
        |d| {
            d.abort_edit().unwrap();
        },
        |d| {
            d.commit_edit().unwrap();
        },
        |d| {
            d.abort_edit().unwrap();
            d.begin_edit().unwrap();
            d.add_shape(shape()).unwrap();
        },
        |d| {
            d.commit_edit().unwrap();
            assert!(d.undo());
        },
        |d| {
            d.commit_edit().unwrap();
            assert!(d.undo());
            assert!(d.redo());
        },
        |d| {
            let other = d.plan_commit_edit().unwrap().unwrap();
            d.commit_edit_with_plan(other, false).unwrap();
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut d = Document::new(image()).unwrap();
        d.add_shape(shape()).unwrap();
        d.mark_saved();
        d.begin_edit().unwrap();
        d.add_shape(shape()).unwrap();
        let plan = d.plan_commit_edit().unwrap().unwrap();
        mutate(&mut d);
        let before = plan_observation(&d);
        assert_eq!(
            d.commit_edit_with_plan(plan, true),
            Err(DocumentError::StaleHistoryPlan),
            "mutation {index}"
        );
        assert_eq!(plan_observation(&d), before, "mutation {index}");
    }
}

#[test]
fn snapshot_plan_is_bound_to_one_document_and_survives_rejected_operations() {
    let mut d = Document::new(image()).unwrap();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let plan = d.plan_commit_edit().unwrap().unwrap();
    let mut cloned = d.clone();
    let before = plan_observation(&cloned);
    assert_eq!(
        cloned.commit_edit_with_plan(plan, true),
        Err(DocumentError::StaleHistoryPlan)
    );
    assert_eq!(plan_observation(&cloned), before);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    let before = plan_observation(&d);
    assert_eq!(d.begin_edit(), Err(DocumentError::TransactionAlreadyOpen));
    assert_eq!(d.remove_shape(99), Err(DocumentError::InvalidShapeIndex));
    assert!(!d.mark_saved());
    assert!(!d.try_undo().unwrap());
    assert!(!d.try_redo().unwrap());
    assert_eq!(plan_observation(&d), before);
    d.commit_edit_with_plan(plan, false).unwrap();
}

#[test]
fn suspended_snapshot_restores_original_document_until_approved_apply() {
    let mut d = Document::new(image()).unwrap();
    d.add_shape(shape()).unwrap();
    d.mark_saved();
    let old_image = d.image_arc();
    let old_revision = Arc::clone(&d.current.revision);
    let old_shapes = d.shapes().to_vec();
    let undo_before = d.undo.clone();
    let redo_before = d.redo.clone();
    d.begin_edit().unwrap();
    d.replace_image(Image::new(1, 1, vec![42; 4]).unwrap())
        .unwrap();
    d.add_shape(shape()).unwrap();
    let new_image = d.image_arc();
    let plan = d.plan_commit_edit().unwrap().unwrap();
    let planned_bytes = plan.history_bytes();
    let plan = d.suspend_edit_with_plan(plan).unwrap();
    assert!(d.image_arc().ptr_eq(&old_image));
    assert!(Arc::ptr_eq(&d.current.revision, &old_revision));
    assert_eq!(d.shapes(), old_shapes);
    assert!(!d.is_dirty());
    assert!(d.staged_before.is_none());
    assert_eq!(d.undo, undo_before);
    assert_eq!(d.redo, redo_before);
    d.commit_edit_with_plan(plan, false).unwrap();
    assert!(d.image_arc().ptr_eq(&new_image));
    assert_eq!(d.shapes().len(), 2);
    assert!(d.is_dirty());
    assert_eq!(d.history_bytes(), planned_bytes);
    assert!(d.undo());
    assert!(d.image_arc().ptr_eq(&old_image));
    assert_eq!(d.shapes(), old_shapes);
    assert!(!d.is_dirty());
}

#[test]
fn dropping_suspended_plan_cancels_edit_preserving_redo_and_saved_state() {
    let mut d = Document::new(image()).unwrap();
    d.add_shape(shape()).unwrap();
    assert!(d.undo());
    d.mark_saved();
    let original = d.current.clone();
    let undo_before = d.undo.clone();
    let redo_before = d.redo.clone();
    let bytes_before = d.history_bytes();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let plan = d.plan_commit_edit().unwrap().unwrap();
    let plan = d.suspend_edit_with_plan(plan).unwrap();
    drop(plan);
    assert_eq!(d.current, original);
    assert!(Arc::ptr_eq(&d.current.revision, &original.revision));
    assert_eq!(d.undo, undo_before);
    assert_eq!(d.redo, redo_before);
    assert_eq!(d.history_bytes(), bytes_before);
    assert!(!d.is_dirty());
    assert!(d.can_redo());
    assert!(d.staged_before.is_none());
}

#[test]
fn suspended_plan_rejects_intervening_edits_and_stale_suspend_is_atomic() {
    let mut d = Document::new(image()).unwrap();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let stale = d.plan_commit_edit().unwrap().unwrap();
    d.add_shape(shape()).unwrap();
    let before = plan_observation(&d);
    assert!(matches!(
        d.suspend_edit_with_plan(stale),
        Err(DocumentError::StaleHistoryPlan)
    ));
    assert_eq!(plan_observation(&d), before);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    let plan = d.suspend_edit_with_plan(plan).unwrap();
    d.add_shape(shape()).unwrap();
    let before = plan_observation(&d);
    assert_eq!(
        d.commit_edit_with_plan(plan, true),
        Err(DocumentError::StaleHistoryPlan)
    );
    assert_eq!(plan_observation(&d), before);
}

#[test]
fn rejecting_suspended_eviction_leaves_original_state_and_histories_intact() {
    let mut d = Document::new(image()).unwrap();
    for _ in 0..12 {
        d.add_shape(shape()).unwrap();
    }
    d.set_history_budget(d.history_bytes());
    d.mark_saved();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let plan = d.plan_commit_edit().unwrap().unwrap();
    assert!(plan.undo_entries_to_evict() > 0);
    let plan = d.suspend_edit_with_plan(plan).unwrap();
    let before = plan_observation(&d);
    assert_eq!(
        d.commit_edit_with_plan(plan, false),
        Err(DocumentError::HistoryEvictionNotApproved)
    );
    assert_eq!(plan_observation(&d), before);
    assert_eq!(d.shapes().len(), 12);
    assert!(!d.is_dirty());
}

#[test]
fn suspended_plan_is_invalidated_by_save_navigation_budget_and_new_transaction() {
    let mutations: &[fn(&mut Document)] = &[
        |d| {
            assert!(d.mark_saved());
        },
        |d| {
            d.begin_edit().unwrap();
        },
        |d| {
            d.set_history_budget(d.history_budget());
        },
        |d| {
            assert!(d.undo());
        },
        |d| {
            assert!(d.redo());
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut d = Document::new(image()).unwrap();
        d.add_shape(shape()).unwrap();
        d.add_shape(shape()).unwrap();
        assert!(d.undo()); // Both navigation directions are available.
        d.begin_edit().unwrap();
        d.add_shape(shape()).unwrap();
        let plan = d.plan_commit_edit().unwrap().unwrap();
        let plan = d.suspend_edit_with_plan(plan).unwrap();
        mutate(&mut d);
        let before = plan_observation(&d);
        assert_eq!(
            d.commit_edit_with_plan(plan, true),
            Err(DocumentError::StaleHistoryPlan),
            "mutation {index}"
        );
        assert_eq!(plan_observation(&d), before, "mutation {index}");
    }
}

#[test]
fn prepared_snapshot_accepts_exact_minimum_budget_and_rejects_one_byte_less() {
    let mut d = Document::new(image()).unwrap();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let committed_bytes = d.plan_commit_edit().unwrap().unwrap().history_bytes();
    let inverse_containers = NavigationPlan::reserve(0, 1).unwrap();
    let inverse_bytes = snapshot_storage_bytes(&d.current) + inverse_containers.container_bytes();
    let minimum = committed_bytes.max(inverse_bytes);
    d.set_history_budget(minimum - 1);
    let before = plan_observation(&d);
    assert!(matches!(
        d.plan_commit_edit(),
        Err(DocumentError::HistoryBudgetExceeded)
    ));
    assert_eq!(plan_observation(&d), before);
    d.set_history_budget(minimum);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    assert_eq!(plan.history_bytes(), committed_bytes);
    d.commit_edit_with_plan(plan, false).unwrap();
    assert_eq!(d.history_bytes(), committed_bytes);
    assert!(d.try_undo().unwrap());
    assert_eq!(d.history_bytes(), inverse_bytes);
    assert!(d.try_redo().unwrap());
    assert_eq!(d.history_bytes(), committed_bytes);
}

#[test]
fn eviction_releases_shared_image_only_after_its_last_snapshot_is_removed() {
    let a = Image::new(32, 32, vec![1; 4096]).unwrap();
    let b = Image::new(32, 32, vec![2; 4096]).unwrap();
    let mut d = Document::new(a).unwrap();
    d.add_shape(shape()).unwrap();
    d.add_shape(shape()).unwrap();
    d.replace_image(b).unwrap();
    let old_count = d.undo.len();
    d.set_history_budget(d.history_bytes());
    d.begin_edit().unwrap();
    d.replace_image(image()).unwrap();
    let before = plan_observation(&d);
    let plan = d.plan_commit_edit().unwrap().unwrap();
    assert_eq!(plan.undo_entries_to_evict(), old_count);
    assert_eq!(plan_observation(&d), before);
    let expected = snapshot_storage_bytes(plan.snapshot.as_ref().unwrap())
        + image_allocation_bytes(&plan.snapshot.as_ref().unwrap().image)
        + container_bytes(plan.undo.capacity(), size_of::<State>());
    assert_eq!(plan.history_bytes(), expected);
    d.commit_edit_with_plan(plan, true).unwrap();
    assert_eq!(d.undo.len(), 1);
    assert_eq!(d.history_bytes(), expected);
}

#[test]
fn suspended_redo_discard_requires_approval_and_preserves_original_on_rejection() {
    for approve in [false, true] {
        let mut d = Document::new(image()).unwrap();
        d.add_shape(shape()).unwrap();
        d.add_shape(shape()).unwrap();
        assert!(d.undo());
        d.mark_saved();
        d.begin_edit().unwrap();
        d.edit_shape(0, |shape| shape.offset(1.0, 1.0)).unwrap();
        let plan = d.plan_commit_edit().unwrap().unwrap();
        assert_eq!(plan.undo_entries_to_evict(), 0);
        assert_eq!(plan.redo_entries_to_discard(), 1);
        let plan = d.suspend_edit_with_plan(plan).unwrap();
        let before = plan_observation(&d);
        if approve {
            assert!(d.commit_edit_with_plan(plan, true).unwrap());
            assert!(!d.can_redo());
            assert!(d.is_dirty());
            assert_eq!(d.shapes()[0].bounds().x, 3.0);
        } else {
            assert_eq!(
                d.commit_edit_with_plan(plan, false),
                Err(DocumentError::HistoryEvictionNotApproved)
            );
            assert_eq!(plan_observation(&d), before);
            assert!(d.can_redo());
            assert!(!d.is_dirty());
            assert_eq!(d.shapes()[0].bounds().x, 2.0);
        }
    }
}

#[test]
fn planning_rejects_image_edit_when_its_inverse_alone_exceeds_budget() {
    let mut d = Document::with_history_budget(image(), 4096).unwrap();
    d.mark_saved();
    d.begin_edit().unwrap();
    d.replace_image(Image::new(128, 128, vec![9; 128 * 128 * 4]).unwrap())
        .unwrap();
    let before = plan_observation(&d);
    assert!(matches!(
        d.plan_commit_edit(),
        Err(DocumentError::HistoryBudgetExceeded)
    ));
    assert_eq!(plan_observation(&d), before);
    d.abort_edit().unwrap();
    assert_eq!(d.image(), &image());
    assert!(!d.is_dirty());
}

#[test]
fn planned_image_edit_preserves_all_approved_steps_through_undo_and_redo() {
    let mut d = Document::new(image()).unwrap();
    for _ in 0..12 {
        d.add_shape(shape()).unwrap();
    }
    d.set_history_budget(d.history_bytes());
    let original = d.image_arc();
    let original_shapes = d.shapes().to_vec();
    d.begin_edit().unwrap();
    d.add_shape(Shape::new(ShapeKind::Text {
        location: Point::new(0.0, 0.0),
        text: "extra".to_string(),
    }))
    .unwrap();
    let plan = d.plan_commit_edit().unwrap().unwrap();
    let retained = d.undo.len() + 1 - plan.undo_entries_to_evict();
    assert!(plan.undo_entries_to_evict() > 0);
    let plan = d.suspend_edit_with_plan(plan).unwrap();
    d.commit_edit_with_plan(plan, true).unwrap();
    let new_shapes = d.shapes().to_vec();
    assert_eq!(d.undo.len(), retained);
    assert_eq!(d.redo.len(), 0);
    assert!(d.try_undo().unwrap());
    assert!(d.image_arc().ptr_eq(&original));
    assert_eq!(d.shapes(), original_shapes);
    assert_eq!(d.undo.len(), retained - 1);
    assert_eq!(d.redo.len(), 1);
    assert!(d.history_bytes() <= d.history_budget());
    assert!(d.try_redo().unwrap());
    assert_eq!(d.shapes(), new_shapes);
    assert_eq!(d.undo.len(), retained);
    assert_eq!(d.redo.len(), 0);
    assert!(d.history_bytes() <= d.history_budget());
}

fn fallback_document_with_both_histories() -> Document {
    let mut d = Document::with_history_budget(image(), 8192).unwrap();
    for _ in 0..3 {
        d.add_shape(shape()).unwrap();
    }
    assert!(d.undo());
    assert!(d.mark_saved());
    d
}

fn stage_oversized_inverse(d: &mut Document) {
    d.begin_edit().unwrap();
    d.replace_image(Image::new(128, 128, vec![9; 128 * 128 * 4]).unwrap())
        .unwrap();
}

// Suspend intentionally invalidates old plans but must restore everything else.
fn observation_without_generation(d: &Document) -> (String, Vec<usize>) {
    let mut observation = plan_observation(d);
    observation.1[0] = 0;
    observation
}

#[test]
fn fallback_planning_preserves_no_transaction_no_op_and_undoable_branch_behavior() {
    let mut d = fallback_document_with_both_histories();
    let before = plan_observation(&d);
    assert!(matches!(
        d.plan_commit_edit_with_fallback(),
        Err(DocumentError::NoTransaction)
    ));
    assert_eq!(plan_observation(&d), before);
    d.begin_edit().unwrap();
    let before = plan_observation(&d);
    assert!(d.plan_commit_edit_with_fallback().unwrap().is_none());
    assert_eq!(plan_observation(&d), before);
    d.add_shape(shape()).unwrap();
    let before = plan_observation(&d);
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    assert_eq!(plan.mode(), HistoryCommitMode::Undoable);
    assert_eq!(plan.redo_entries_to_discard(), 1);
    assert_eq!(
        d.commit_edit_with_approval(plan, CommitApproval::PreserveHistory),
        Err(DocumentError::HistoryEvictionNotApproved)
    );
    assert_eq!(plan_observation(&d), before);
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    d.commit_edit_with_approval(plan, CommitApproval::DiscardOldHistory)
        .unwrap();
    assert!(d.try_undo().unwrap());
    assert!(d.try_redo().unwrap());
}

#[test]
fn fallback_reports_all_history_loss_and_keeps_strict_planning_unchanged() {
    let mut d = fallback_document_with_both_histories();
    let undo_count = d.undo.len();
    let redo_count = d.redo.len();
    stage_oversized_inverse(&mut d);
    let before = plan_observation(&d);
    assert!(matches!(
        d.plan_commit_edit(),
        Err(DocumentError::HistoryBudgetExceeded)
    ));
    assert_eq!(plan_observation(&d), before);
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    assert_eq!(plan_observation(&d), before);
    assert_eq!(plan.mode(), HistoryCommitMode::WithoutUndo);
    assert_eq!(plan.undo_entries_to_evict(), undo_count);
    assert_eq!(plan.redo_entries_to_discard(), redo_count);
    assert_eq!(plan.history_bytes(), 0);
    assert_eq!(plan.history_budget(), 8192);
    assert!(plan.snapshot.is_none());
    assert_eq!(plan.undo.capacity(), 0);
}

#[test]
fn fallback_requires_distinct_consent_even_with_empty_history() {
    for with_history in [false, true] {
        let mut d = if with_history {
            fallback_document_with_both_histories()
        } else {
            Document::with_history_budget(image(), 8192).unwrap()
        };
        stage_oversized_inverse(&mut d);
        for approval in [
            CommitApproval::PreserveHistory,
            CommitApproval::DiscardOldHistory,
        ] {
            let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
            let before = plan_observation(&d);
            assert_eq!(
                d.commit_edit_with_approval(plan, approval),
                Err(DocumentError::IrreversibleEditNotApproved)
            );
            assert_eq!(plan_observation(&d), before);
        }
        for allow_history_discard in [false, true] {
            let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
            let before = plan_observation(&d);
            assert_eq!(
                d.commit_edit_with_plan(plan, allow_history_discard),
                Err(DocumentError::IrreversibleEditNotApproved)
            );
            assert_eq!(plan_observation(&d), before);
        }
        d.abort_edit().unwrap();
    }
}

#[test]
fn dropping_or_declining_suspended_fallback_restores_exact_original_document() {
    for decline in [false, true] {
        let mut d = fallback_document_with_both_histories();
        let original = observation_without_generation(&d);
        stage_oversized_inverse(&mut d);
        let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
        let plan = d.suspend_edit_with_plan(plan).unwrap();
        assert_eq!(observation_without_generation(&d), original);
        let before = plan_observation(&d);
        if decline {
            assert_eq!(
                d.commit_edit_with_approval(plan, CommitApproval::DiscardOldHistory),
                Err(DocumentError::IrreversibleEditNotApproved)
            );
        } else {
            drop(plan);
        }
        assert_eq!(plan_observation(&d), before);
        assert!(!d.is_dirty());
        assert!(d.can_redo());
        assert!(d.staged_before.is_none());
    }
}

#[test]
fn approved_fallback_installs_new_baseline_without_undo_or_redo_allocations() {
    for suspend in [false, true] {
        let mut d = fallback_document_with_both_histories();
        let saved = Arc::clone(d.saved_revision.as_ref().unwrap());
        stage_oversized_inverse(&mut d);
        let expected_image = d.image_arc();
        let expected_shapes = d.shapes().to_vec();
        let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
        let plan = if suspend {
            d.suspend_edit_with_plan(plan).unwrap()
        } else {
            plan
        };
        assert!(d
            .commit_edit_with_approval(plan, CommitApproval::CommitWithoutUndo)
            .unwrap());
        assert!(expected_image.ptr_eq(&d.image_arc()));
        assert_eq!(d.shapes(), expected_shapes);
        assert!(Arc::ptr_eq(&saved, d.saved_revision.as_ref().unwrap()));
        assert!(!Arc::ptr_eq(&saved, &d.current.revision));
        assert!(d.is_dirty());
        assert!(d.staged_before.is_none());
        assert_eq!(d.history_budget(), 8192);
        assert_eq!(d.history_bytes(), 0);
        assert_eq!(d.undo.capacity(), 0);
        assert_eq!(d.redo.capacity(), 0);
        let before = plan_observation(&d);
        assert_eq!(d.check_undo(), Ok(false));
        assert_eq!(d.check_redo(), Ok(false));
        assert_eq!(d.try_undo(), Ok(false));
        assert_eq!(d.try_redo(), Ok(false));
        assert_eq!(plan_observation(&d), before);
        d.begin_edit().unwrap();
        d.add_shape(shape()).unwrap();
        let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
        assert_eq!(plan.mode(), HistoryCommitMode::Undoable);
        d.commit_edit_with_approval(plan, CommitApproval::PreserveHistory)
            .unwrap();
        assert!(d.try_undo().unwrap());
        assert_eq!(d.shapes(), expected_shapes);
        assert!(!d.try_undo().unwrap()); // Cannot cross the irreversible baseline.
        assert!(d.try_redo().unwrap());
    }
}

#[test]
fn fallback_applies_to_oversized_old_snapshot_as_well_as_oversized_inverse() {
    let mut d =
        Document::with_history_budget(Image::new(128, 128, vec![7; 128 * 128 * 4]).unwrap(), 8192)
            .unwrap();
    d.begin_edit().unwrap();
    d.replace_image(image()).unwrap();
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    assert_eq!(plan.mode(), HistoryCommitMode::WithoutUndo);
    d.commit_edit_with_approval(plan, CommitApproval::CommitWithoutUndo)
        .unwrap();
    assert_eq!(d.image(), &image());
    assert!(!d.try_undo().unwrap());
    assert!(!d.try_redo().unwrap());
}

#[test]
fn fallback_preserves_budget_boundary_and_prefers_undoable_eviction() {
    let mut d = Document::new(image()).unwrap();
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let snapshot = d.staged_before.as_ref().unwrap().clone();
    let committed = snapshot_storage_bytes(&snapshot)
        + NavigationPlan::reserve(1, 0).unwrap().container_bytes();
    let inverse = snapshot_storage_bytes(&d.current)
        + NavigationPlan::reserve(0, 1).unwrap().container_bytes();
    let minimum = committed.max(inverse);
    for (budget, mode) in [
        (minimum - 1, HistoryCommitMode::WithoutUndo),
        (minimum, HistoryCommitMode::Undoable),
        (minimum + 1, HistoryCommitMode::Undoable),
    ] {
        d.set_history_budget(budget);
        let before = plan_observation(&d);
        let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
        assert_eq!(plan.mode(), mode);
        assert_eq!(plan_observation(&d), before);
    }
    d.abort_edit().unwrap();
    d.set_history_budget(DEFAULT_HISTORY_BUDGET);
    for _ in 0..12 {
        d.add_shape(shape()).unwrap();
    }
    d.set_history_budget(d.history_bytes());
    d.begin_edit().unwrap();
    d.add_shape(shape()).unwrap();
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    assert_eq!(plan.mode(), HistoryCommitMode::Undoable);
    assert!(plan.undo_entries_to_evict() > 0);
    d.commit_edit_with_approval(plan, CommitApproval::CommitWithoutUndo)
        .unwrap();
    assert!(d.try_undo().unwrap()); // Stronger approval must not force fallback.
    assert!(d.try_redo().unwrap());
}

#[test]
fn fallback_rejects_stale_edits_budget_changes_clones_and_new_transactions() {
    let mutations: &[fn(&mut Document)] = &[
        |d| {
            d.add_shape(shape()).unwrap();
        },
        |d| {
            d.set_history_budget(d.history_budget());
        },
        |d| {
            d.abort_edit().unwrap();
            d.begin_edit().unwrap();
            d.add_shape(shape()).unwrap();
        },
        |d| {
            let p = d.plan_commit_edit_with_fallback().unwrap().unwrap();
            d.commit_edit_with_approval(p, CommitApproval::CommitWithoutUndo)
                .unwrap();
        },
    ];
    for mutate in mutations {
        let mut d = fallback_document_with_both_histories();
        stage_oversized_inverse(&mut d);
        let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
        mutate(&mut d);
        let before = plan_observation(&d);
        assert_eq!(
            d.commit_edit_with_approval(plan, CommitApproval::CommitWithoutUndo),
            Err(DocumentError::StaleHistoryPlan)
        );
        assert_eq!(plan_observation(&d), before);
    }
    let mut d = fallback_document_with_both_histories();
    stage_oversized_inverse(&mut d);
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    let mut cloned = d.clone();
    let before = plan_observation(&cloned);
    assert_eq!(
        cloned.commit_edit_with_approval(plan, CommitApproval::CommitWithoutUndo),
        Err(DocumentError::StaleHistoryPlan)
    );
    assert_eq!(plan_observation(&cloned), before);
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    let plan = d.suspend_edit_with_plan(plan).unwrap();
    assert!(d.mark_saved());
    let before = plan_observation(&d);
    assert_eq!(
        d.commit_edit_with_approval(plan, CommitApproval::CommitWithoutUndo),
        Err(DocumentError::StaleHistoryPlan)
    );
    assert_eq!(plan_observation(&d), before);
}

struct HistoryReservationFailureGuard;
impl HistoryReservationFailureGuard {
    fn after(successful_reservations: usize) -> Self {
        HISTORY_RESERVATION_FAILURE_AFTER.with(|after| after.set(Some(successful_reservations)));
        Self
    }
}
impl Drop for HistoryReservationFailureGuard {
    fn drop(&mut self) {
        HISTORY_RESERVATION_FAILURE_AFTER.with(|after| after.set(None));
    }
}

#[test]
fn allocation_failure_at_every_fallback_reservation_is_propagated_atomically() {
    let mut d = fallback_document_with_both_histories();
    stage_oversized_inverse(&mut d);
    HISTORY_RESERVATION_COUNT.with(|count| count.set(0));
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    let reservations = HISTORY_RESERVATION_COUNT.with(|count| count.get());
    assert_eq!(plan.mode(), HistoryCommitMode::WithoutUndo);
    assert!(reservations > 5); // Includes the final minimum-budget attempt.
    drop(plan);
    let before = plan_observation(&d);
    for fail_after in 0..reservations {
        let _guard = HistoryReservationFailureGuard::after(fail_after);
        assert!(
            matches!(
                d.plan_commit_edit_with_fallback(),
                Err(DocumentError::AllocationFailed)
            ),
            "reservation {fail_after}"
        );
        assert_eq!(plan_observation(&d), before);
    }
    let plan = d.plan_commit_edit_with_fallback().unwrap().unwrap();
    assert_eq!(plan.mode(), HistoryCommitMode::WithoutUndo);
    assert_eq!(plan_observation(&d), before);
    let mut impossible: Vec<u8> = Vec::new();
    assert_eq!(
        reserve_history(|| impossible.try_reserve(usize::MAX)),
        Err(DocumentError::AllocationFailed)
    );
}

#[test]
fn existing_handle_constructor_preserves_pixels_and_rejected_ownership() {
    let source = ImageHandle::from_image(image());
    let pixels = source.pixels.as_ptr();
    let d = Document::from_image_handle(source.clone()).unwrap();
    assert!(source.ptr_eq(&d.image_arc()));
    assert_eq!(d.image().pixels.as_ptr(), pixels);

    let invalid = ImageHandle::from_image(Image {
        width: 3,
        height: 2,
        pixels: vec![0; 4],
    });
    let original = invalid.clone();
    let pixels = invalid.pixels.as_ptr();
    let (error, returned) = Document::from_image_handle(invalid).unwrap_err();
    assert_eq!(error, DocumentError::InvalidImage);
    assert!(returned.ptr_eq(&original));
    assert_eq!(returned.pixels.as_ptr(), pixels);
    assert_eq!(returned.pixels, vec![0; 4]);
}

#[test]
fn imported_handle_keeps_budget_in_history_clones_and_external_peeks() {
    use crate::{
        import_admission::ImportAdmission,
        resource_budget::{AdmissionClass, ResourceBudget},
    };

    let source = image();
    let pixels = source.pixels.as_ptr();
    let reserved = source.pixels.capacity();
    let budget = ResourceBudget::new(reserved, 0).unwrap();
    let owner = ImportAdmission::new(budget.clone());
    let lease = budget
        .try_acquire(reserved, AdmissionClass::Normal)
        .unwrap();
    let imported = ImageHandle::from_imported(owner.try_begin(source, lease).unwrap().complete());
    let mut d = Document::from_image_handle(imported.clone()).unwrap();
    assert!(d.image_arc().ptr_eq(&imported));
    assert_eq!(d.image().pixels.as_ptr(), pixels);
    drop(imported);
    drop(owner);

    d.crop(CropRect {
        x: 1,
        y: 0,
        width: 2,
        height: 2,
    })
    .unwrap();
    assert!(matches!(d.image_arc(), ImageHandle::Untracked(_)));
    let undo_image = d.undo_image_arc().unwrap();
    assert_eq!(undo_image.pixels.as_ptr(), pixels);
    assert!(matches!(undo_image, ImageHandle::Imported(_)));
    assert_eq!(budget.snapshot().used_bytes, reserved);
    let mut clone = d.clone();
    assert!(clone.undo_image_arc().unwrap().ptr_eq(&undo_image));
    drop(undo_image);
    drop(d);
    assert_eq!(budget.snapshot().used_bytes, reserved);

    assert!(clone.undo());
    assert_eq!(clone.image().pixels.as_ptr(), pixels);
    assert!(clone.redo());
    let retained = clone.undo_image_arc().unwrap();
    drop(clone);
    assert_eq!(budget.snapshot().used_bytes, reserved);
    assert_eq!(retained.pixels.as_ptr(), pixels);
    drop(retained);
    assert_eq!(budget.snapshot().used_bytes, 0);
}

fn assert_same_crop_baseline(document: &Document, before: &Document) {
    assert!(document.current.image.ptr_eq(&before.current.image));
    assert!(Arc::ptr_eq(
        &document.current.revision,
        &before.current.revision
    ));
    assert_eq!(document.current.shapes, before.current.shapes);
    assert_eq!(document.is_dirty(), before.is_dirty());
    assert_eq!(document.undo, before.undo);
    assert_eq!(document.redo, before.redo);
    for (after, before) in document
        .undo
        .iter()
        .chain(&document.redo)
        .zip(before.undo.iter().chain(&before.redo))
    {
        assert!(after.image.ptr_eq(&before.image));
        assert!(Arc::ptr_eq(&after.revision, &before.revision));
    }
    assert!(document.staged_before.is_none());
}

fn tracked_crop_document(budget: &crate::resource_budget::ResourceBudget) -> Document {
    use crate::{import_admission::ImportAdmission, resource_budget::AdmissionClass};
    let source = image();
    let lease = budget
        .try_acquire(source.pixels.capacity(), AdmissionClass::Normal)
        .unwrap();
    let admission = ImportAdmission::new(budget.clone());
    let tracked = admission.try_begin(source, lease).unwrap().complete();
    Document::from_image_handle(ImageHandle::from_imported(tracked)).unwrap()
}

fn budget_crop_rect() -> CropRect {
    CropRect {
        x: 1,
        y: 0,
        width: 2,
        height: 2,
    }
}

fn budget_frame_options() -> FrameOptions {
    FrameOptions {
        width: 1,
        color: Color::rgba(23, 45, 67, 128),
        shadow: None,
    }
}

#[test]
fn frame_budget_and_direct_history_refusal_preserve_revision_and_both_histories() {
    use crate::resource_budget::{AdmissionClass, ResourceBudget};
    let budget = ResourceBudget::new(128, 0).unwrap();
    let mut document = tracked_crop_document(&budget);
    document.add_shape(shape()).unwrap();
    document.add_shape(shape()).unwrap();
    assert!(document.undo());
    assert!(document.mark_saved());
    let before = document.clone();
    let used = budget.snapshot().used_bytes;
    let blocker = budget
        .try_acquire(128 - used, AdmissionClass::Normal)
        .unwrap();
    assert!(matches!(
        document.add_frame_with_budget(budget_frame_options(), &budget),
        Err(DocumentError::Image(ImageEditError::Budget(_)))
    ));
    assert_same_crop_baseline(&document, &before);
    drop(blocker);
    document.history_budget = 0;
    assert_eq!(
        document.add_frame_with_budget(budget_frame_options(), &budget),
        Err(DocumentError::HistoryBudgetExceeded)
    );
    assert_same_crop_baseline(&document, &before);
    assert_eq!(budget.snapshot().used_bytes, used);
}

#[test]
fn budgeted_frame_bitmap_failure_history_failure_and_decline_restore_exact_baseline() {
    use crate::resource_budget::ResourceBudget;
    for failure in 0..3 {
        let budget = ResourceBudget::new(128, 0).unwrap();
        let mut document = tracked_crop_document(&budget);
        document.add_shape(shape()).unwrap();
        document.add_shape(shape()).unwrap();
        assert!(document.undo()); // Keep both undo and redo populated.
        assert!(document.mark_saved());
        let before = document.clone();
        let used = budget.snapshot().used_bytes;
        document.begin_edit().unwrap();
        document
            .add_frame_with_budget(budget_frame_options(), &budget)
            .unwrap();
        assert!(budget.snapshot().used_bytes > used);
        match failure {
            0 => {
                // Models native bitmap construction returning Err before the
                // history dialog. The staged output alone is abandoned.
                document.abort_edit().unwrap();
            }
            1 => {
                let _guard = HistoryReservationFailureGuard::after(0);
                assert!(matches!(
                    document.plan_commit_edit_with_fallback(),
                    Err(DocumentError::AllocationFailed)
                ));
                document.abort_edit().unwrap();
            }
            _ => {
                let plan = document.plan_commit_edit_with_fallback().unwrap().unwrap();
                let plan = document.suspend_edit_with_plan(plan).unwrap();
                assert_same_crop_baseline(&document, &before);
                // A declined history confirmation drops the suspended output.
                drop(plan);
            }
        }
        assert_same_crop_baseline(&document, &before);
        assert_eq!(budget.snapshot().used_bytes, used);
    }
}

#[test]
fn budgeted_frame_lease_follows_current_undo_redo_plans_and_external_clones() {
    use crate::resource_budget::ResourceBudget;
    let budget = ResourceBudget::new(128, 0).unwrap();
    let mut document = tracked_crop_document(&budget);
    let source = document.image_arc();
    let source_bytes = source.pixels.capacity();
    document.begin_edit().unwrap();
    document
        .add_frame_with_budget(budget_frame_options(), &budget)
        .unwrap();
    let frame = document.image_arc();
    let frame_bytes = frame.pixels.capacity();
    let plan = document.plan_commit_edit().unwrap().unwrap();
    let plan = document.suspend_edit_with_plan(plan).unwrap();
    assert!(document.image_arc().ptr_eq(&source));
    assert_eq!(budget.snapshot().used_bytes, source_bytes + frame_bytes);
    document
        .commit_edit_with_approval(plan, CommitApproval::PreserveHistory)
        .unwrap();
    assert!(document.image_arc().ptr_eq(&frame));
    assert!(document.try_undo().unwrap());
    assert!(document.redo_image_arc().unwrap().ptr_eq(&frame));
    assert_eq!(budget.snapshot().used_bytes, source_bytes + frame_bytes);
    assert!(document.try_redo().unwrap());
    assert_eq!(budget.snapshot().used_bytes, source_bytes + frame_bytes);
    let external = document.clone();
    drop(document);
    drop(frame);
    drop(source);
    assert_eq!(budget.snapshot().used_bytes, source_bytes + frame_bytes);
    drop(external);
    assert_eq!(budget.snapshot().used_bytes, 0);
}

#[test]
fn crop_budget_and_direct_history_refusal_preserve_revision_and_both_histories() {
    use crate::resource_budget::{AdmissionClass, ResourceBudget};
    let budget = ResourceBudget::new(128, 0).unwrap();
    let mut document = tracked_crop_document(&budget);
    document.add_shape(shape()).unwrap();
    document.add_shape(shape()).unwrap();
    assert!(document.undo());
    assert!(document.mark_saved());
    let before = document.clone();
    let used = budget.snapshot().used_bytes;
    let blocker = budget
        .try_acquire(128 - used, AdmissionClass::Normal)
        .unwrap();
    assert!(matches!(
        document.crop_with_budget(budget_crop_rect(), &budget),
        Err(DocumentError::Image(ImageEditError::Budget(_)))
    ));
    assert_same_crop_baseline(&document, &before);
    drop(blocker);
    document.history_budget = 0;
    assert_eq!(
        document.crop_with_budget(budget_crop_rect(), &budget),
        Err(DocumentError::HistoryBudgetExceeded)
    );
    assert_same_crop_baseline(&document, &before);
    assert_eq!(budget.snapshot().used_bytes, used);
}

#[test]
fn budgeted_crop_bitmap_failure_history_failure_and_decline_restore_exact_baseline() {
    use crate::resource_budget::ResourceBudget;
    for failure in 0..3 {
        let budget = ResourceBudget::new(128, 0).unwrap();
        let mut document = tracked_crop_document(&budget);
        document.add_shape(shape()).unwrap();
        document.add_shape(shape()).unwrap();
        assert!(document.undo()); // Keep both undo and redo populated.
        assert!(document.mark_saved());
        let before = document.clone();
        let used = budget.snapshot().used_bytes;
        document.begin_edit().unwrap();
        document
            .crop_with_budget(budget_crop_rect(), &budget)
            .unwrap();
        assert!(budget.snapshot().used_bytes > used);
        match failure {
            0 => {
                // Models native bitmap construction returning Err before the
                // history dialog. The staged output alone is abandoned.
                document.abort_edit().unwrap();
            }
            1 => {
                let _guard = HistoryReservationFailureGuard::after(0);
                assert!(matches!(
                    document.plan_commit_edit_with_fallback(),
                    Err(DocumentError::AllocationFailed)
                ));
                document.abort_edit().unwrap();
            }
            _ => {
                let plan = document.plan_commit_edit_with_fallback().unwrap().unwrap();
                let plan = document.suspend_edit_with_plan(plan).unwrap();
                assert_same_crop_baseline(&document, &before);
                // A declined history confirmation drops the suspended output.
                drop(plan);
            }
        }
        assert_same_crop_baseline(&document, &before);
        assert_eq!(budget.snapshot().used_bytes, used);
    }
}

#[test]
fn budgeted_crop_lease_follows_current_undo_redo_plans_and_external_clones() {
    use crate::resource_budget::ResourceBudget;
    let budget = ResourceBudget::new(128, 0).unwrap();
    let mut document = tracked_crop_document(&budget);
    let source = document.image_arc();
    let source_bytes = source.pixels.capacity();
    document.begin_edit().unwrap();
    document
        .crop_with_budget(budget_crop_rect(), &budget)
        .unwrap();
    let crop = document.image_arc();
    let crop_bytes = crop.pixels.capacity();
    let plan = document.plan_commit_edit().unwrap().unwrap();
    let plan = document.suspend_edit_with_plan(plan).unwrap();
    assert!(document.image_arc().ptr_eq(&source));
    assert_eq!(budget.snapshot().used_bytes, source_bytes + crop_bytes);
    document
        .commit_edit_with_approval(plan, CommitApproval::PreserveHistory)
        .unwrap();
    assert!(document.image_arc().ptr_eq(&crop));
    assert!(document.try_undo().unwrap());
    assert!(document.redo_image_arc().unwrap().ptr_eq(&crop));
    assert_eq!(budget.snapshot().used_bytes, source_bytes + crop_bytes);
    assert!(document.try_redo().unwrap());
    assert_eq!(budget.snapshot().used_bytes, source_bytes + crop_bytes);
    let external = document.clone();
    drop(document);
    drop(crop);
    drop(source);
    assert_eq!(budget.snapshot().used_bytes, source_bytes + crop_bytes);
    drop(external);
    assert_eq!(budget.snapshot().used_bytes, 0);
}

#[test]
fn seeded_capture_cursor_belongs_to_the_initial_state() {
    use crate::{cursor_snapshot::CursorPixels, shapes::CursorSprite};
    let cursor = || {
        Shape::new(ShapeKind::Cursor {
            data: CursorSprite::new(
                Point::new(1.0, 0.0),
                1,
                1,
                CursorPixels::PremultipliedRgba(vec![0, 0, 0, 255]),
            )
            .unwrap(),
        })
    };
    // An auto-saved capture is clean; seeding the cursor keeps it clean and
    // adds no history step, so Undo cannot remove it.
    let mut document = Document::new(image()).unwrap();
    document.mark_saved();
    document.seed_initial_shapes(vec![cursor()]).unwrap();
    assert_eq!(document.shapes().len(), 1);
    assert!(!document.is_dirty());
    assert!(!document.can_undo());
    assert!(!document.undo());
    assert_eq!(document.shapes().len(), 1);
    // The cursor is an ordinary object afterwards: movable and removable.
    document
        .edit_shape(0, |shape| shape.offset(1.0, 1.0))
        .unwrap();
    document.remove_shape(0).unwrap();
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.shapes()[0].bounds().x, 1.0);
    // Only a fresh, unedited document can be seeded.
    assert_eq!(
        document.seed_initial_shapes(vec![cursor()]),
        Err(DocumentError::NotInitialState)
    );
    let mut edited = Document::new(image()).unwrap();
    edited.add_shape(shape()).unwrap();
    assert!(edited.undo());
    assert_eq!(
        edited.seed_initial_shapes(vec![cursor()]),
        Err(DocumentError::NotInitialState)
    );
}
