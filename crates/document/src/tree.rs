//! Addressing paragraphs inside the (shallow) block tree.
//!
//! A [`ParaRef`] names a paragraph by section and a path of alternating
//! block / child-flow indices: `[block, child, block, child, block, ...]`.
//! The editor works with the flat reading-order list from
//! [`Document::paragraphs`], so ordering positions is just comparing indices.

use std::sync::Arc;

use crate::model::{Block, Document, Paragraph, Section};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParaRef {
    pub section: usize,
    pub path: Vec<u32>,
}

impl ParaRef {
    /// True when `other` is the next block in the same flow as `self`.
    pub fn is_next_sibling(&self, other: &ParaRef) -> bool {
        let n = self.path.len();
        self.section == other.section
            && n == other.path.len()
            && self.path[..n - 1] == other.path[..n - 1]
            && self.path[n - 1] + 1 == other.path[n - 1]
    }

    pub fn same_flow(&self, other: &ParaRef) -> bool {
        let n = self.path.len();
        self.section == other.section && n == other.path.len() && self.path[..n - 1] == other.path[..n - 1]
    }
}

impl Document {
    /// All paragraphs in reading order.
    pub fn paragraphs(&self) -> Vec<ParaRef> {
        let mut out = Vec::new();
        for (si, s) in self.sections.iter().enumerate() {
            collect(&s.blocks, si, &mut Vec::new(), &mut out);
        }
        out
    }

    pub fn paragraph(&self, r: &ParaRef) -> Option<&Paragraph> {
        match block_at(&self.sections.get(r.section)?.blocks, &r.path)? {
            Block::Paragraph(p) => Some(p),
            _ => None,
        }
    }

    pub fn paragraph_mut(&mut self, r: &ParaRef) -> Option<&mut Paragraph> {
        let section = Arc::make_mut(self.sections.get_mut(r.section)?);
        match block_at_mut(&mut section.blocks, &r.path)? {
            Block::Paragraph(p) => Some(p),
            _ => None,
        }
    }

    /// The flow (block list) containing the paragraph, plus its index in it.
    pub fn flow_mut(&mut self, r: &ParaRef) -> Option<(&mut Vec<Block>, usize)> {
        let section = Arc::make_mut(self.sections.get_mut(r.section)?);
        let (last, parent) = r.path.split_last()?;
        let mut flow = &mut section.blocks;
        for pair in parent.chunks(2) {
            flow = flow.get_mut(pair[0] as usize)?.child_mut(pair[1] as usize)?;
        }
        Some((flow, *last as usize))
    }

    pub fn section_mut(&mut self, i: usize) -> &mut Section {
        Arc::make_mut(&mut self.sections[i])
    }
}

fn collect(blocks: &[Block], section: usize, prefix: &mut Vec<u32>, out: &mut Vec<ParaRef>) {
    for (bi, b) in blocks.iter().enumerate() {
        prefix.push(bi as u32);
        match b {
            Block::Paragraph(_) => out.push(ParaRef { section, path: prefix.clone() }),
            _ => {
                for (ci, child) in b.children().into_iter().enumerate() {
                    prefix.push(ci as u32);
                    collect(child, section, prefix, out);
                    prefix.pop();
                }
            }
        }
        prefix.pop();
    }
}

fn block_at<'a>(blocks: &'a [Block], path: &[u32]) -> Option<&'a Block> {
    let b = blocks.get(*path.first()? as usize)?;
    if path.len() == 1 {
        return Some(b);
    }
    let child = *b.children().get(path[1] as usize)?;
    block_at(child, &path[2..])
}

fn block_at_mut<'a>(blocks: &'a mut [Block], path: &[u32]) -> Option<&'a mut Block> {
    let b = blocks.get_mut(*path.first()? as usize)?;
    if path.len() == 1 {
        return Some(b);
    }
    let child = b.child_mut(path[1] as usize)?;
    block_at_mut(child, &path[2..])
}
