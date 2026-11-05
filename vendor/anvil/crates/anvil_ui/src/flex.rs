//! Taffy, wired to Anvil's text measuring. Phase 0 of ANVIL-UI-2.md: the dependency and one
//! proven path (a row of text that sizes to its content and wraps). The element tree builds on
//! this in Phase 1.

pub use taffy;

#[cfg(test)]
mod tests {
    use taffy::prelude::*;

    use crate::text::{FontRole, TextSystem, WEIGHT_REGULAR};

    /// A flex row of two text leaves measured by `TextSystem`: each gets its text's width, and
    /// a narrow container wraps them onto two lines.
    #[test]
    fn text_leaves_size_to_their_content() {
        let mut text = TextSystem::new();
        let mut tree: TaffyTree<&str> = TaffyTree::new();
        let a = tree.new_leaf_with_context(Style::default(), "Battery").unwrap();
        let b = tree.new_leaf_with_context(Style::default(), "Pair phone now").unwrap();
        let row = tree
            .new_with_children(
                Style { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, gap: Size { width: length(8.0), height: length(4.0) }, ..Default::default() },
                &[a, b],
            )
            .unwrap();
        // Taffy 0.14 hands the measure function a LayoutInput and wants a LayoutOutput back.
        let mut measure = |input: taffy::LayoutInput, _: NodeId, ctx: Option<&mut &str>, _: &Style| {
            let s = ctx.map(|c| *c).unwrap_or("");
            let width = input.known_dimensions.width.or(match input.available_space.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let key = text.layout(s, 16.0, WEIGHT_REGULAR, FontRole::Ui, width);
            taffy::LayoutOutput::from_outer_size(Size { width: text.width(&key), height: text.lines(&key) as f32 * 20.0 })
        };
        tree.compute_layout_with_measure(row, Size { width: AvailableSpace::Definite(400.0), height: AvailableSpace::MaxContent }, &mut measure).unwrap();
        let (la, lb) = (*tree.layout(a).unwrap(), *tree.layout(b).unwrap());
        assert!(la.size.width > 40.0 && la.size.width < 90.0, "{:?}", la.size);
        assert_eq!(lb.location.x, la.size.width + 8.0, "side by side with the gap");
        // Too narrow for both: the second wraps under the first.
        tree.compute_layout_with_measure(row, Size { width: AvailableSpace::Definite(120.0), height: AvailableSpace::MaxContent }, &mut measure).unwrap();
        let lb = *tree.layout(b).unwrap();
        assert_eq!(lb.location.x, 0.0);
        assert!(lb.location.y >= 20.0, "{:?}", lb.location);
    }
}
