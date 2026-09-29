//! Stage 2: split a region into flows, column groups and tables by finding
//! vertical whitespace gutters that persist across consecutive line bands.
//!
//! Sweep bands top to bottom. A band with internal gaps opens a *candidate*;
//! following bands keep it open while the gutters survive (no line crosses
//! them). When the candidate closes it is classified from measurable
//! evidence:
//!
//! * lines on each side share baselines (row-aligned) and the last column is
//!   right-aligned  -> not columns: tabbed rows inside the flow (resume dates)
//! * row-aligned otherwise                                -> table
//! * independent vertical flows with >= 2 lines per column -> columns
//! * anything else                                        -> ordinary flow

use crate::lines::Line;

#[derive(Debug)]
pub enum Zone {
    Flow { lines: Vec<usize>, x0: f32, x1: f32 },
    Columns { cols: Vec<ColumnZone> },
    Table { cols: Vec<(f32, f32)>, rows: Vec<Vec<Vec<usize>>> },
}

#[derive(Debug)]
pub struct ColumnZone {
    pub x0: f32,
    pub x1: f32,
    pub zones: Vec<Zone>,
}

impl Zone {
    /// All line indices in this zone (any depth).
    pub fn line_ids(&self, out: &mut Vec<usize>) {
        match self {
            Zone::Flow { lines, .. } => out.extend(lines),
            Zone::Columns { cols } => cols.iter().flat_map(|c| &c.zones).for_each(|z| z.line_ids(out)),
            Zone::Table { rows, .. } => rows.iter().flatten().for_each(|c| out.extend(c)),
        }
    }
}

struct Band {
    ids: Vec<usize>,
    /// Merged occupied x-intervals, sorted.
    occupied: Vec<(f32, f32)>,
}

impl Band {
    fn internal_gaps(&self) -> Vec<(f32, f32)> {
        self.occupied.windows(2).map(|w| (w[0].1, w[1].0)).collect()
    }

    /// Intersects gutter `g` with this band's free space; `None` if a line
    /// crosses it or the remainder is too narrow.
    fn narrow(&self, g: (f32, f32), min: f32) -> Option<(f32, f32)> {
        let (mut a, mut b) = g;
        for &(o0, o1) in &self.occupied {
            if o1 <= a || o0 >= b {
                continue;
            }
            // Occupied interval overlaps the gutter: keep the larger side.
            if o0 - a >= b - o1 { b = o0 } else { a = o1 }
        }
        (b - a >= min).then_some((a, b))
    }
}

pub fn segment(lines: &[Line], mut ids: Vec<usize>, x0: f32, x1: f32, depth: u32) -> Vec<Zone> {
    if ids.is_empty() {
        return Vec::new();
    }
    ids.sort_by(|&a, &b| lines[a].top().total_cmp(&lines[b].top()));
    let mut sizes: Vec<f32> = ids.iter().map(|&i| lines[i].size).collect();
    sizes.sort_by(f32::total_cmp);
    let em = sizes[sizes.len() / 2];
    let min_gutter = 0.6 * em;

    // Bands: transitive vertical overlap of line boxes.
    let mut bands: Vec<Band> = Vec::new();
    let mut band_bottom = f32::MIN;
    for &i in &ids {
        let l = &lines[i];
        if l.top() < band_bottom - 0.05 * l.size
            && let Some(b) = bands.last_mut()
        {
            b.ids.push(i);
            band_bottom = band_bottom.max(l.bottom());
        } else {
            bands.push(Band { ids: vec![i], occupied: Vec::new() });
            band_bottom = l.bottom();
        }
    }
    for b in &mut bands {
        let mut iv: Vec<(f32, f32)> = b.ids.iter().map(|&i| (lines[i].x0, lines[i].x1)).collect();
        iv.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (s, e) in iv {
            match b.occupied.last_mut() {
                Some(last) if s <= last.1 + min_gutter => last.1 = last.1.max(e),
                _ => b.occupied.push((s, e)),
            }
        }
    }

    let mut zones = Vec::new();
    let mut flow: Vec<usize> = Vec::new();
    let flush_flow = |flow: &mut Vec<usize>, zones: &mut Vec<Zone>| {
        if !flow.is_empty() {
            zones.push(Zone::Flow { lines: std::mem::take(flow), x0, x1 });
        }
    };

    // Candidate: gutters, first band index, member lines.
    type Candidate = (Vec<(f32, f32)>, usize, Vec<usize>);
    let mut cand: Option<Candidate> = None;
    let mut i = 0;
    while i <= bands.len() {
        let band = bands.get(i);
        if let Some((gutters, _, members)) = &mut cand {
            if let Some(band) = band {
                let surviving: Vec<_> = gutters.iter().filter_map(|&g| band.narrow(g, min_gutter)).collect();
                // A band crossing only some gutters ends a table (rows with
                // all columns) but merely narrows a column candidate.
                let partial = !surviving.is_empty() && surviving.len() < gutters.len();
                let table_ends =
                    partial && matches!(classify(lines, gutters, members.clone(), depth), Ok(Zone::Table { .. }));
                if !surviving.is_empty() && !table_ends {
                    *gutters = surviving;
                    members.extend(&band.ids);
                    i += 1;
                    continue;
                }
            }
            let (gutters, start, members) = cand.take().expect("candidate");
            match classify(lines, &gutters, members, depth) {
                Ok(zone) => {
                    flush_flow(&mut flow, &mut zones);
                    zones.push(zone);
                }
                Err(_) => {
                    // Backtrack: the opening band is ordinary flow; resume
                    // the sweep right after it.
                    flow.extend(&bands[start].ids);
                    i = start + 1;
                    continue;
                }
            }
        }
        let Some(band) = band else { break };
        let gaps: Vec<_> = band.internal_gaps().into_iter().filter(|g| g.1 - g.0 >= min_gutter).collect();
        if gaps.is_empty() {
            flow.extend(&band.ids);
        } else {
            cand = Some((gaps, i, band.ids.clone()));
        }
        i += 1;
    }
    flush_flow(&mut flow, &mut zones);
    for z in &mut zones {
        if let Zone::Flow { lines: ids, .. } = z {
            ids.sort_by(|&a, &b| lines[a].baseline.total_cmp(&lines[b].baseline).then(lines[a].x0.total_cmp(&lines[b].x0)));
        }
    }
    zones
}

fn same_row(a: &Line, b: &Line) -> bool {
    (a.baseline - b.baseline).abs() <= 0.25 * a.size.min(b.size)
}

fn classify(lines: &[Line], gutters: &[(f32, f32)], members: Vec<usize>, depth: u32) -> Result<Zone, Vec<usize>> {
    let mut gutters = gutters.to_vec();
    gutters.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut cols: Vec<Vec<usize>> = vec![Vec::new(); gutters.len() + 1];
    for &i in &members {
        let c = gutters.iter().take_while(|g| lines[i].x0 >= g.1 - 0.01).count();
        cols[c].push(i);
    }
    if cols.iter().any(Vec::is_empty) {
        return Err(members);
    }
    let col_of = |i: usize| cols.iter().position(|c| c.contains(&i)).expect("assigned");
    let aligned = members
        .iter()
        .filter(|&&i| members.iter().any(|&j| col_of(j) != col_of(i) && same_row(&lines[i], &lines[j])))
        .count();
    let rowish = aligned as f32 >= 0.8 * members.len() as f32;

    // A right-aligned last column whose lines all sit on rows of other
    // columns is a set of tab-aligned attachments (dates, locations).
    let last = cols.last().expect("non-empty");
    let spread = last.iter().map(|&i| lines[i].x1).fold(f32::MIN, f32::max)
        - last.iter().map(|&i| lines[i].x1).fold(f32::MAX, f32::min);
    let partnered = last
        .iter()
        .filter(|&&i| members.iter().any(|&j| col_of(j) != cols.len() - 1 && same_row(&lines[i], &lines[j])))
        .count();
    if spread <= 2.0 && partnered as f32 >= 0.8 * last.len() as f32 {
        return Err(members);
    }

    if rowish {
        let mut rows: Vec<Vec<Vec<usize>>> = Vec::new();
        let mut ids = members.clone();
        ids.sort_by(|&a, &b| lines[a].baseline.total_cmp(&lines[b].baseline));
        let mut row_baseline = f32::MIN;
        for i in ids {
            if rows.is_empty() || !same_row_bl(row_baseline, &lines[i]) {
                rows.push(vec![Vec::new(); cols.len()]);
                row_baseline = lines[i].baseline;
            }
            rows.last_mut().expect("row")[col_of(i)].push(i);
        }
        if rows.len() < 2 {
            return Err(members);
        }
        let bounds = cols
            .iter()
            .map(|c| {
                (
                    c.iter().map(|&i| lines[i].x0).fold(f32::MAX, f32::min),
                    c.iter().map(|&i| lines[i].x1).fold(f32::MIN, f32::max),
                )
            })
            .collect();
        return Ok(Zone::Table { cols: bounds, rows });
    }

    if depth >= 3 || cols.iter().any(|c| c.len() < 2) {
        return Err(members);
    }
    let cols = cols
        .into_iter()
        .map(|c| {
            let cx0 = c.iter().map(|&i| lines[i].x0).fold(f32::MAX, f32::min);
            let cx1 = c.iter().map(|&i| lines[i].x1).fold(f32::MIN, f32::max);
            ColumnZone { x0: cx0, x1: cx1, zones: segment(lines, c, cx0, cx1, depth + 1) }
        })
        .collect();
    Ok(Zone::Columns { cols })
}

fn same_row_bl(baseline: f32, l: &Line) -> bool {
    (baseline - l.baseline).abs() <= 0.25 * l.size
}
