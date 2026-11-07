//! Dock model of one session (replaces dockview): a grid of groups, each group a stack of panels
//! (tabs). Reads and writes dockview's serialized JSON, so store.json layouts stay compatible with
//! the Tauri build in both directions.
//!
//! Invariant: a branch child of a branch always has the orthogonal axis (as in dockview's gridview).

use serde_json::{Map, Value, json};

/// Side of a group a panel is dropped on (`None` = into the group, as a tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    fn axis(self) -> Axis {
        match self {
            Edge::Left | Edge::Right => Axis::Row,
            Edge::Top | Edge::Bottom => Axis::Column,
        }
    }

    fn after(self) -> bool {
        matches!(self, Edge::Right | Edge::Bottom)
    }
}

/// `Row`: children side by side (dockview HORIZONTAL). `Column`: stacked (VERTICAL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

impl Axis {
    pub fn flip(self) -> Axis {
        match self {
            Axis::Row => Axis::Column,
            Axis::Column => Axis::Row,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Axis::Row => "HORIZONTAL",
            Axis::Column => "VERTICAL",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    pub views: Vec<String>,
    pub active: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(Group),
    Branch(Branch),
}

/// Children with their size along `axis` (any unit: only proportions matter).
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub axis: Axis,
    pub children: Vec<(f32, Node)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dock {
    pub root: Branch,
    /// Group shown alone (dockview "maximized group").
    pub maximized: Option<String>,
    pub active_group: Option<String>,
    /// Serialized dockview panel states, kept as is for compatibility.
    panels: Map<String, Value>,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            root: Branch {
                axis: Axis::Row,
                children: Vec::new(),
            },
            maximized: None,
            active_group: None,
            panels: Map::new(),
        }
    }
}

/// Smallest size a resized child keeps, as a fraction of its branch.
const MIN_FRACTION: f32 = 0.05;

fn new_group(view: &str) -> Group {
    Group {
        id: format!("g-{}", agentdeck_core::logic::deck::new_id()),
        views: vec![view.to_owned()],
        active: view.to_owned(),
    }
}

impl Dock {
    /// Parses a dockview layout. `None` when it is not one (the caller starts from an empty dock).
    pub fn from_json(value: &Value) -> Option<Dock> {
        let grid = value.get("grid")?;
        let axis = match grid.get("orientation")?.as_str()? {
            "HORIZONTAL" => Axis::Row,
            "VERTICAL" => Axis::Column,
            _ => return None,
        };
        let Node::Branch(root) = parse_node(grid.get("root")?, axis)? else {
            return None;
        };
        let mut dock = Dock {
            root,
            maximized: None,
            active_group: value.get("activeGroup").and_then(Value::as_str).map(str::to_owned),
            panels: value.get("panels").and_then(Value::as_object).cloned().unwrap_or_default(),
        };
        dock.normalize();
        Some(dock)
    }

    /// Dockview JSON. Sizes are scaled to `width` x `height` px like dockview's own `toJSON`.
    pub fn to_json(&self, width: f32, height: f32, panel_title: impl Fn(&str) -> String) -> Value {
        let (along, across) = match self.root.axis {
            Axis::Row => (width, height),
            Axis::Column => (height, width),
        };
        let mut root = branch_json(&self.root, along, across);
        root["size"] = json!(across.round());
        let mut panels = Map::new();
        for id in self.views() {
            let mut state = self.panels.get(&id).cloned().unwrap_or_else(|| {
                let browser = id.strip_prefix("browser:");
                json!({
                    "id": id,
                    "contentComponent": if browser.is_some() { "browser" } else { "pane" },
                    "tabComponent": "props.defaultTabComponent",
                    "params": { "paneId": browser.unwrap_or(&id) },
                })
            });
            state["title"] = json!(panel_title(&id));
            panels.insert(id, state);
        }
        let mut out = json!({
            "grid": { "root": root, "width": width.round(), "height": height.round(), "orientation": self.root.axis.name() },
            "panels": panels,
        });
        if let Some(group) = &self.active_group {
            out["activeGroup"] = json!(group);
        }
        out
    }

    /// Every panel id, in tree order (top-left first).
    pub fn views(&self) -> Vec<String> {
        let mut out = Vec::new();
        for_each_group(&self.root, &mut |g| out.extend(g.views.iter().cloned()));
        out
    }

    pub fn groups(&self) -> Vec<&Group> {
        let mut out = Vec::new();
        collect_groups(&self.root, &mut out);
        out
    }

    pub fn group_of(&self, view: &str) -> Option<&Group> {
        self.groups().into_iter().find(|g| g.views.iter().any(|v| v == view))
    }

    pub fn is_empty(&self) -> bool {
        self.root.children.is_empty()
    }

    /// Adds a panel in a new group right of `reference` (or of the last group), like dockview `addPanel`.
    pub fn add_view(&mut self, view: &str, reference: Option<&str>) {
        if self.group_of(view).is_some() {
            return;
        }
        let reference = reference
            .and_then(|r| self.group_of(r))
            .or_else(|| self.groups().last().copied())
            .map(|g| g.id.clone());
        let group = new_group(view);
        self.active_group = Some(group.id.clone());
        match reference {
            Some(target) => self.insert_beside(&target, Edge::Right, group),
            None => self.root.children.push((1.0, Node::Leaf(group))),
        }
    }

    /// Removes a panel; its group disappears when empty.
    pub fn remove_view(&mut self, view: &str) {
        self.panels.remove(view);
        remove_from(&mut self.root, view);
        self.normalize();
    }

    /// Drag and drop: into `target` group as a tab (`edge` None) or in a new group on that side.
    pub fn move_view(&mut self, view: &str, target: &str, edge: Option<Edge>) {
        let Some(source) = self.group_of(view).cloned() else { return };
        if source.id == target && (edge.is_none() || source.views.len() == 1) {
            return;
        }
        if self.groups().iter().all(|g| g.id != target) {
            return;
        }
        let state = self.panels.remove(view);
        remove_from(&mut self.root, view);
        // The target group survives: it is not the source, or the source kept other views.
        match edge {
            None => {
                if let Some(g) = group_mut(&mut self.root, target) {
                    g.views.push(view.to_owned());
                    g.active = view.to_owned();
                }
                self.active_group = Some(target.to_owned());
            }
            Some(edge) => {
                let group = new_group(view);
                self.active_group = Some(group.id.clone());
                self.insert_beside(target, edge, group);
            }
        }
        if let Some(state) = state {
            self.panels.insert(view.to_owned(), state);
        }
        self.normalize();
    }

    /// Makes `view` the visible tab of its group and its group the active one.
    pub fn set_active(&mut self, view: &str) -> bool {
        let Some(id) = self.group_of(view).map(|g| g.id.clone()) else { return false };
        if let Some(g) = group_mut(&mut self.root, &id) {
            g.active = view.to_owned();
        }
        self.active_group = Some(id);
        true
    }

    pub fn is_maximized(&self, view: &str) -> bool {
        self.maximized.is_some() && self.maximized.as_deref() == self.group_of(view).map(|g| g.id.as_str())
    }

    /// Maximizes the group of `view`, or restores it when already maximized.
    pub fn toggle_maximize(&mut self, view: &str) {
        if self.is_maximized(view) {
            self.maximized = None;
        } else {
            self.maximized = self.group_of(view).map(|g| g.id.clone());
        }
    }

    /// dockview `revealPane`: leaves another group's maximized state and selects the panel.
    pub fn reveal(&mut self, view: &str) -> bool {
        if self.maximized.is_some() && !self.is_maximized(view) {
            self.maximized = None;
        }
        self.set_active(view)
    }

    /// Resizes the branch at `path`: the sash after child `index` moves to `fraction` (0..1 of the branch).
    pub fn resize(&mut self, path: &[usize], index: usize, fraction: f32) {
        let Some(branch) = branch_mut(&mut self.root, path) else { return };
        if index + 1 >= branch.children.len() {
            return;
        }
        let total: f32 = branch.children.iter().map(|c| c.0).sum();
        let before: f32 = branch.children[..index].iter().map(|c| c.0).sum();
        let pair = branch.children[index].0 + branch.children[index + 1].0;
        let min = total * MIN_FRACTION;
        if pair <= 2.0 * min {
            return;
        }
        let first = (fraction * total - before).clamp(min, pair - min);
        branch.children[index].0 = first;
        branch.children[index + 1].0 = pair - first;
    }

    /// Auto grid (lib/gridLayout.ts): `views` row by row, rows of equal height, one panel per group.
    pub fn grid(&mut self, views: &[String]) {
        let mut next = views.iter();
        let rows = agentdeck_core::logic::layout::grid_rows(views.len())
            .into_iter()
            .map(|count| {
                let cells = next
                    .by_ref()
                    .take(count)
                    .map(|v| {
                        let group = Group {
                            id: format!("grid-{v}"),
                            views: vec![v.clone()],
                            active: v.clone(),
                        };
                        (1.0, Node::Leaf(group))
                    })
                    .collect();
                (1.0, Node::Branch(Branch { axis: Axis::Row, children: cells }))
            })
            .collect();
        self.root = Branch {
            axis: Axis::Column,
            children: rows,
        };
        self.maximized = None;
        self.panels.retain(|id, _| views.contains(id));
        self.active_group = self.active_group.take().filter(|g| self.groups().iter().any(|x| &x.id == g));
    }

    /// Deck.tsx sync: adds missing panels (a browser right of its agent, others right of the last),
    /// closes panels no longer wanted. Returns true when something changed.
    pub fn sync(&mut self, wanted: &[String]) -> bool {
        let mut changed = false;
        for id in wanted {
            if self.group_of(id).is_none() {
                let reference = id
                    .strip_prefix("browser:")
                    .filter(|pane| self.group_of(pane).is_some())
                    .map(str::to_owned);
                self.add_view(id, reference.as_deref());
                changed = true;
            }
        }
        for id in self.views() {
            if !wanted.contains(&id) {
                self.remove_view(&id);
                changed = true;
            }
        }
        changed
    }

    /// Inserts `group` on `edge` of the group `target` (splitting it when the axis differs).
    fn insert_beside(&mut self, target: &str, edge: Edge, group: Group) {
        let Some(path) = path_of(&self.root, target) else {
            self.root.children.push((1.0, Node::Leaf(group)));
            return;
        };
        let (index, parent_path) = path.split_last().map(|(i, p)| (*i, p.to_vec())).unwrap_or((0, Vec::new()));
        let Some(parent) = branch_mut(&mut self.root, &parent_path) else { return };
        if parent.axis == edge.axis() {
            let half = parent.children[index].0 / 2.0;
            parent.children[index].0 = half;
            let at = if edge.after() { index + 1 } else { index };
            parent.children.insert(at, (half, Node::Leaf(group)));
            return;
        }
        let (size, old) = parent.children.remove(index);
        let new = (1.0, Node::Leaf(group));
        let children = if edge.after() { vec![(1.0, old), new] } else { vec![new, (1.0, old)] };
        parent.children.insert(
            index,
            (
                size,
                Node::Branch(Branch {
                    axis: edge.axis(),
                    children,
                }),
            ),
        );
    }

    /// Drops empty groups and branches, collapses single-child branches, keeps references valid.
    fn normalize(&mut self) {
        normalize_branch(&mut self.root);
        // A root with one branch child takes that child's place (its axis flips).
        while self.root.children.len() == 1 {
            let Node::Branch(_) = &self.root.children[0].1 else { break };
            let Some((_, Node::Branch(inner))) = self.root.children.pop() else { break };
            self.root = inner;
        }
        let groups: Vec<String> = self.groups().iter().map(|g| g.id.clone()).collect();
        if self.maximized.as_ref().is_some_and(|m| !groups.contains(m)) {
            self.maximized = None;
        }
        if self.active_group.as_ref().is_some_and(|a| !groups.contains(a)) {
            self.active_group = groups.last().cloned();
        }
    }
}

fn parse_node(value: &Value, axis: Axis) -> Option<Node> {
    match value.get("type")?.as_str()? {
        "leaf" => {
            let data = value.get("data")?;
            let views: Vec<String> = data
                .get("views")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            let active = data
                .get("activeView")
                .and_then(Value::as_str)
                .filter(|a| views.iter().any(|v| v == a))
                .map(str::to_owned)
                .or_else(|| views.first().cloned())?;
            let id = data.get("id").and_then(Value::as_str)?.to_owned();
            Some(Node::Leaf(Group { id, views, active }))
        }
        "branch" => {
            let children = value
                .get("data")?
                .as_array()?
                .iter()
                .filter_map(|child| {
                    let size = child.get("size").and_then(Value::as_f64).unwrap_or(1.0).max(1.0) as f32;
                    Some((size, parse_node(child, axis.flip())?))
                })
                .collect();
            Some(Node::Branch(Branch { axis, children }))
        }
        _ => None,
    }
}

fn branch_json(branch: &Branch, along: f32, across: f32) -> Value {
    let total: f32 = branch.children.iter().map(|c| c.0).sum::<f32>().max(f32::EPSILON);
    let data: Vec<Value> = branch
        .children
        .iter()
        .map(|(size, node)| {
            let px = (size / total * along).round();
            match node {
                Node::Leaf(g) => json!({
                    "type": "leaf",
                    "data": { "views": g.views, "activeView": g.active, "id": g.id },
                    "size": px,
                }),
                Node::Branch(b) => {
                    let mut v = branch_json(b, across, px);
                    v["size"] = json!(px);
                    v
                }
            }
        })
        .collect();
    json!({ "type": "branch", "data": data })
}

fn for_each_group(branch: &Branch, f: &mut impl FnMut(&Group)) {
    for (_, node) in &branch.children {
        match node {
            Node::Leaf(g) => f(g),
            Node::Branch(b) => for_each_group(b, f),
        }
    }
}

fn collect_groups<'a>(branch: &'a Branch, out: &mut Vec<&'a Group>) {
    for (_, node) in &branch.children {
        match node {
            Node::Leaf(g) => out.push(g),
            Node::Branch(b) => collect_groups(b, out),
        }
    }
}

/// Child indices from the root to the group `id`.
fn path_of(branch: &Branch, id: &str) -> Option<Vec<usize>> {
    for (i, (_, node)) in branch.children.iter().enumerate() {
        match node {
            Node::Leaf(g) if g.id == id => return Some(vec![i]),
            Node::Branch(b) => {
                if let Some(mut rest) = path_of(b, id) {
                    rest.insert(0, i);
                    return Some(rest);
                }
            }
            Node::Leaf(_) => {}
        }
    }
    None
}

pub fn branch_at<'a>(branch: &'a Branch, path: &[usize]) -> Option<&'a Branch> {
    match path.split_first() {
        None => Some(branch),
        Some((i, rest)) => match &branch.children.get(*i)?.1 {
            Node::Branch(b) => branch_at(b, rest),
            Node::Leaf(_) => None,
        },
    }
}

fn branch_mut<'a>(branch: &'a mut Branch, path: &[usize]) -> Option<&'a mut Branch> {
    match path.split_first() {
        None => Some(branch),
        Some((i, rest)) => match &mut branch.children.get_mut(*i)?.1 {
            Node::Branch(b) => branch_mut(b, rest),
            Node::Leaf(_) => None,
        },
    }
}

fn group_mut<'a>(branch: &'a mut Branch, id: &str) -> Option<&'a mut Group> {
    for (_, node) in &mut branch.children {
        match node {
            Node::Leaf(g) if g.id == id => return Some(g),
            Node::Branch(b) => {
                if let Some(g) = group_mut(b, id) {
                    return Some(g);
                }
            }
            Node::Leaf(_) => {}
        }
    }
    None
}

fn remove_from(branch: &mut Branch, view: &str) {
    for (_, node) in &mut branch.children {
        match node {
            Node::Leaf(g) => {
                if let Some(i) = g.views.iter().position(|v| v == view) {
                    g.views.remove(i);
                    if g.active == view {
                        g.active = g.views.get(i.saturating_sub(1)).or(g.views.first()).cloned().unwrap_or_default();
                    }
                }
            }
            Node::Branch(b) => remove_from(b, view),
        }
    }
}

fn normalize_branch(branch: &mut Branch) {
    let mut children = Vec::with_capacity(branch.children.len());
    for (size, node) in std::mem::take(&mut branch.children) {
        match node {
            Node::Leaf(g) if g.views.is_empty() => {}
            Node::Leaf(g) => children.push((size, Node::Leaf(g))),
            Node::Branch(mut b) => {
                normalize_branch(&mut b);
                match b.children.len() {
                    0 => {}
                    // One child left: it takes the branch's place. A leaf keeps the slot size;
                    // a branch child has our axis, so its children are spliced in at our level.
                    1 => match b.children.pop() {
                        Some((_, Node::Leaf(g))) => children.push((size, Node::Leaf(g))),
                        Some((_, Node::Branch(inner))) => {
                            let total: f32 = inner.children.iter().map(|c| c.0).sum::<f32>().max(f32::EPSILON);
                            children.extend(inner.children.into_iter().map(|(s, n)| (s / total * size, n)));
                        }
                        None => {}
                    },
                    _ => children.push((size, Node::Branch(b))),
                }
            }
        }
    }
    branch.children = children;
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_a_real_dockview_layout() {
        let saved = json!({"grid": {"root": {"type": "branch", "data": [{"type": "branch", "data": [
            {"type": "leaf", "data": {"views": ["a"], "activeView": "a", "id": "grid-a"}, "size": 1068}], "size": 812}],
            "size": 1068}, "width": 1068, "height": 812, "orientation": "VERTICAL"},
            "panels": {"a": {"id": "a", "contentComponent": "pane", "params": {"paneId": "a"}, "title": "Claude Code"}},
            "activeGroup": "grid-a"});
        let dock = Dock::from_json(&saved).expect("dockview layout");
        assert_eq!(dock.views(), ids(&["a"]));
        // The single-cell row collapses into a leaf of the root.
        assert_eq!(dock.root.axis, Axis::Column);
        let back = dock.to_json(1000.0, 800.0, |_| "T".into());
        assert_eq!(back["panels"]["a"]["contentComponent"], "pane");
        assert_eq!(back["grid"]["orientation"], "VERTICAL");
        assert_eq!(Dock::from_json(&back).map(|d| d.views()), Some(ids(&["a"])));
    }

    #[test]
    fn grid_matches_grid_layout() {
        let mut dock = Dock::default();
        dock.grid(&ids(&["a", "b", "c"]));
        let json = dock.to_json(1200.0, 800.0, |_| "T".into());
        let rows = json["grid"]["root"]["data"].as_array().cloned().unwrap_or_default();
        assert_eq!(json["grid"]["orientation"], "VERTICAL");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["size"], 400.0);
        assert_eq!(rows[0]["data"][1]["data"]["id"], "grid-b");
        assert_eq!(rows[0]["data"][1]["size"], 600.0);
        assert_eq!(dock.views(), ids(&["a", "b", "c"]));
    }

    #[test]
    fn add_split_tab_and_remove() {
        let mut dock = Dock::default();
        assert!(dock.sync(&ids(&["a", "b"])));
        assert_eq!(dock.root.children.len(), 2);
        assert!(!dock.sync(&ids(&["a", "b"])));

        let a = dock.group_of("a").map(|g| g.id.clone()).unwrap_or_default();
        dock.move_view("b", &a, Some(Edge::Bottom));
        // b is now under a: one column branch inside the row root collapses to a column root.
        assert_eq!(dock.root.axis, Axis::Column);
        assert_eq!(dock.views(), ids(&["a", "b"]));

        dock.move_view("b", &a, None);
        assert_eq!(dock.groups().len(), 1);
        assert_eq!(dock.group_of("a").map(|g| g.active.clone()).as_deref(), Some("b"));

        dock.toggle_maximize("a");
        assert!(dock.is_maximized("b"));
        dock.remove_view("b");
        dock.remove_view("a");
        assert!(dock.is_empty());
        assert_eq!(dock.maximized, None);
    }

    #[test]
    fn browser_goes_right_of_its_agent() {
        let mut dock = Dock::default();
        dock.sync(&ids(&["a", "b"]));
        dock.sync(&ids(&["a", "browser:a", "b"]));
        assert_eq!(dock.views(), ids(&["a", "browser:a", "b"]));
        let json = dock.to_json(900.0, 600.0, |_| "Browser".into());
        assert_eq!(json["panels"]["browser:a"]["contentComponent"], "browser");
        assert_eq!(json["panels"]["browser:a"]["params"]["paneId"], "a");
    }

    #[test]
    fn resize_clamps() {
        let mut dock = Dock::default();
        dock.sync(&ids(&["a", "b"]));
        dock.resize(&[], 0, 0.25);
        let sizes: Vec<f32> = dock.root.children.iter().map(|c| c.0).collect();
        assert!((sizes[0] / (sizes[0] + sizes[1]) - 0.25).abs() < 1e-4);
        dock.resize(&[], 0, -3.0);
        assert!(dock.root.children[0].0 > 0.0);
    }
}
