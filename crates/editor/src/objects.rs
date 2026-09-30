//! Block objects (images): insertion, resizing and deletion.

use std::sync::Arc;

use document::{Block, ImageBlock, ImageResource, Paragraph, ParagraphStyle};

use crate::{EditKind, Editor, Pos, Selection};

impl Editor {
    /// Inserts an image below the caret's paragraph at its natural size
    /// (96 dpi), scaled down to fit `max_width`.
    pub fn insert_image(&mut self, image: Arc<ImageResource>, max_width: f32) {
        if self.paras.is_empty() || image.width_px == 0 || image.height_px == 0 {
            return;
        }
        self.checkpoint(EditKind::Other);
        let p = self.sel.focus;
        let r = self.paras[p.para].clone();
        let style = self.paragraph(p.para).style_at(p.offset).clone();
        let natural = image.width_px as f32 * 0.75;
        let width = natural.min(max_width.max(24.0));
        let height = width * image.height_px as f32 / image.width_px as f32;
        let at_start = p.offset == 0 && !self.paragraph(p.para).is_empty();
        let Some((flow, idx)) = self.doc.flow_mut(&r) else { return };
        let at = if at_start { idx } else { idx + 1 };
        flow.insert(at, Block::Image(ImageBlock { space_before: 6.0, x: 0.0, width, height, image }));
        if !matches!(flow.get(at + 1), Some(Block::Paragraph(_))) {
            let mut after = Paragraph::new("", style, ParagraphStyle::default());
            after.style.space_before = 6.0;
            flow.insert(at + 1, Block::Paragraph(after));
        }
        self.refresh();
    }

    pub fn image_size(&self, section: usize, path: &[u32]) -> Option<(f32, f32)> {
        match self.doc.block(section, path)? {
            Block::Image(i) => Some((i.width, i.height)),
            _ => None,
        }
    }

    /// Resizes the image at `path` to `width`, keeping its aspect ratio.
    pub fn resize_image(&mut self, section: usize, path: &[u32], width: f32) {
        let Some((w, h)) = self.image_size(section, path) else { return };
        if w <= 0.0 || (width - w).abs() < 0.1 {
            return;
        }
        self.checkpoint(EditKind::Other);
        if let Some(Block::Image(img)) = self.doc.block_mut(section, path) {
            img.width = width.max(12.0);
            img.height = img.width * h / w;
        }
        self.refresh();
    }

    /// Deletes a non-paragraph block (image, rule, table…) at `path`,
    /// leaving a paragraph in its place if the flow would become empty.
    pub fn delete_block(&mut self, section: usize, path: &[u32]) {
        let Some(&last) = path.last() else { return };
        self.checkpoint(EditKind::Other);
        let body = self.body_style();
        if let Some(flow) = self.doc.flow_of_mut(section, path)
            && (last as usize) < flow.len()
        {
            flow.remove(last as usize);
            if !flow.iter().any(|b| matches!(b, Block::Paragraph(_))) {
                flow.insert(last as usize, Block::Paragraph(Paragraph::new("", body, ParagraphStyle::default())));
            }
        }
        self.paras = self.doc.paragraphs();
        // Put the caret where the block was (the next paragraph after it).
        if let Some(i) = self.paras.iter().position(|r| r.section == section && r.path.as_slice() >= path) {
            self.sel = Selection::caret(Pos::new(i, 0));
        }
        self.refresh();
    }
}

impl Editor {
    /// Page size and margins of the section holding the caret.
    pub fn page_setup(&self) -> (document::Size, document::Margins) {
        let si = self.paras.get(self.sel.focus.para).map_or(0, |r| r.section);
        self.doc.sections.get(si).map_or((document::Size::new(612.0, 792.0), document::Margins::default()), |s| (s.page_size, s.margins))
    }

    /// Applies a page size and margins to the caret's page, or every page.
    pub fn set_page_setup(&mut self, size: document::Size, margins: document::Margins, whole_document: bool) {
        self.checkpoint(EditKind::Other);
        let current = self.paras.get(self.sel.focus.para).map_or(0, |r| r.section);
        for i in 0..self.doc.sections.len() {
            if whole_document || i == current {
                let s = self.doc.section_mut(i);
                s.page_size = size;
                s.margins = margins;
            }
        }
        self.refresh();
    }
}
