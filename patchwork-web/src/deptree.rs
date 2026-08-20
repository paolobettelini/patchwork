use std::collections::{HashMap, HashSet, VecDeque};

use leptos::ev::MouseEvent;
use leptos::prelude::*;
use patchwork_registry_types::{
    RegistryDependencyGraph, RegistryDependencyGraphEdge, RegistryDependencyGraphNode,
    RegistryDependencyKind, RegistryProjectKind, RegistryProjectRef,
};

const NODE_WIDTH: f64 = 230.0;
const NODE_HEIGHT: f64 = 82.0;
const COLUMN_GAP: f64 = 280.0;
const ROW_GAP: f64 = 150.0;
const MAX_TREE_LAYOUT_NODES: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DependencyTreeView {
    Compact,
    Tree,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DependencyNodeKey {
    view: DependencyTreeView,
    expand_modpacks: bool,
    index: usize,
}

#[derive(Clone, PartialEq)]
struct DependencyTreeLayout {
    nodes: Vec<DependencyTreeNode>,
    edges: Vec<DependencyTreeEdge>,
    root_index: usize,
    truncated: bool,
}

#[derive(Clone, PartialEq)]
struct DependencyTreeNode {
    index: usize,
    project_kind: RegistryProjectKind,
    project_id: String,
    title: String,
    description: String,
    version: Option<String>,
    available: bool,
    cycle: bool,
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, PartialEq)]
struct DependencyTreeEdge {
    from: usize,
    to: usize,
    kind: RegistryDependencyKind,
}

#[component]
pub(crate) fn DependencyTreePage(
    graph: Signal<Option<RegistryDependencyGraph>>,
    pending: Signal<bool>,
    error: Signal<Option<String>>,
    on_open_project: Callback<RegistryProjectRef>,
) -> impl IntoView {
    view! {
        <div class="deptree-page">
            <Show when=move || pending.get() && graph.get().is_none()>
                <div class="deptree-status">
                    <span class="project-page-spinner"></span>
                    <strong>"Loading dependency graph..."</strong>
                </div>
            </Show>

            <Show when=move || error.get().is_some()>
                <div class="deptree-error">
                    <strong>"Could not render dependency graph"</strong>
                    <p>{move || error.get().unwrap_or_default()}</p>
                </div>
            </Show>

            {move || {
                graph.get().map(|graph| {
                    view! {
                        <DependencyTreeCanvas graph on_open_project />
                    }
                })
            }}
        </div>
    }
}

#[component]
fn DependencyTreeCanvas(
    graph: RegistryDependencyGraph,
    on_open_project: Callback<RegistryProjectRef>,
) -> impl IntoView {
    let view_mode = RwSignal::new(DependencyTreeView::Compact);
    let expand_modpacks = RwSignal::new(false);

    let root = graph
        .nodes
        .get(graph.root_index)
        .cloned()
        .unwrap_or_else(|| RegistryDependencyGraphNode {
            project_kind: RegistryProjectKind::Mod,
            project_id: "unknown".to_owned(),
            title: "Unknown mod".to_owned(),
            description: "Project information is unavailable.".to_owned(),
            version: None,
            available: false,
        });
    let is_modpack_root = root.project_kind == RegistryProjectKind::Modpack;

    let graph_for_layout = graph.clone();

    let layout = Memo::new(move |_| {
        let expand = is_modpack_root && expand_modpacks.get();
        let projected = dependency_graph_projection(&graph_for_layout, expand);
        match view_mode.get() {
            DependencyTreeView::Compact => layout_dependency_graph(&projected),
            DependencyTreeView::Tree => layout_dependency_tree(&projected),
        }
    });

    let (is_dragging, set_is_dragging) = signal(false);
    let (drag_start, set_drag_start) = signal((0.0, 0.0));
    let (offset, set_offset) = signal((0.0, 0.0));

    let node_offsets =
        RwSignal::new(HashMap::<DependencyNodeKey, (f64, f64)>::new());

    let dragging_node = RwSignal::new(None::<DependencyNodeKey>);
    let drag_distance = RwSignal::new(0.0_f64);
    let suppress_next_click = RwSignal::new(false);

    let on_stage_mouse_down = move |event: MouseEvent| {
        event.prevent_default();

        set_is_dragging.set(true);

        set_drag_start.set((
            event.client_x() as f64,
            event.client_y() as f64,
        ));
    };

    let on_mouse_up = move |_| {
        set_is_dragging.set(false);
        dragging_node.set(None);
    };

    let on_mouse_move = move |event: MouseEvent| {
        if !is_dragging.get() {
            return;
        }

        let (start_x, start_y) = drag_start.get();

        let delta_x = event.client_x() as f64 - start_x;
        let delta_y = event.client_y() as f64 - start_y;

        if let Some(node_key) = dragging_node.get() {
            node_offsets.update(|offsets| {
                let node_offset = offsets
                    .entry(node_key)
                    .or_insert((0.0, 0.0));

                node_offset.0 += delta_x;
                node_offset.1 += delta_y;
            });

            drag_distance.update(|distance| {
                *distance +=
                    (delta_x * delta_x + delta_y * delta_y).sqrt();

                if *distance > 5.0 {
                    suppress_next_click.set(true);
                }
            });
        } else {
            set_offset.update(|offset| {
                offset.0 += delta_x;
                offset.1 += delta_y;
            });
        }

        set_drag_start.set((
            event.client_x() as f64,
            event.client_y() as f64,
        ));
    };

    let root_ref = RegistryProjectRef {
        project_kind: root.project_kind,
        project_id: root.project_id.clone(),
    };
    let back_label = format!(
        "← Back to {}",
        project_kind_label(root.project_kind).to_lowercase()
    );

    view! {
        <div
            class="deptree-stage"
            on:mouseup=on_mouse_up
            on:mouseleave=on_mouse_up
            on:mousemove=on_mouse_move
            on:mousedown=on_stage_mouse_down
        >
            <div
                class="deptree-toolbar"
                on:mousedown=move |event| event.stop_propagation()
            >
                <button
                    type="button"
                    class="deptree-back"
                    on:click=move |_| {
                        on_open_project.run(root_ref.clone())
                    }
                >
                    {back_label}
                </button>

                <div>
                    <strong>{root.project_id}</strong>
                    <span>{root.title}</span>
                </div>

                <div class="deptree-toolbar-controls">
                    <Show when=move || is_modpack_root>
                        <button
                            type="button"
                            class=move || {
                                if expand_modpacks.get() {
                                    "deptree-expand-toggle active"
                                } else {
                                    "deptree-expand-toggle"
                                }
                            }
                            aria-pressed=move || expand_modpacks.get().to_string()
                            on:click=move |_| {
                                expand_modpacks.update(|expanded| *expanded = !*expanded)
                            }
                        >
                            "Expand modpacks"
                        </button>
                    </Show>

                    <div
                        class="deptree-view-toggle"
                        aria-label="Dependency graph layout"
                    >
                        <button
                            type="button"
                            class=move || {
                                view_button_class(
                                    view_mode.get(),
                                    DependencyTreeView::Compact,
                                )
                            }
                            on:click=move |_| {
                                view_mode.set(DependencyTreeView::Compact)
                            }
                        >
                            "Compact"
                        </button>

                        <button
                            type="button"
                            class=move || {
                                view_button_class(
                                    view_mode.get(),
                                    DependencyTreeView::Tree,
                                )
                            }
                            on:click=move |_| {
                                view_mode.set(DependencyTreeView::Tree)
                            }
                        >
                            "Tree"
                        </button>
                    </div>
                </div>
            </div>

            <Show when=move || layout.get().truncated>
                <div class="deptree-warning">
                    "Tree view truncated because the expanded graph is very large. Compact view still contains every unique node."
                </div>
            </Show>

            <div
                class="deptree-world"
                style:transform=move || {
                    let (x, y) = offset.get();
                    format!("translate({x}px, {y}px)")
                }
            >
                <svg class="deptree-edges" aria-hidden="true">
                    {move || {
                        let current_layout = layout.get();
                        let current_view = view_mode.get();
                        let nodes = current_layout.nodes;

                        current_layout
                            .edges
                            .into_iter()
                            .filter_map(move |edge| {
                                let from = nodes.get(edge.from)?.clone();
                                let to = nodes.get(edge.to)?.clone();

                                let from_key = DependencyNodeKey {
                                    view: current_view,
                                    expand_modpacks: is_modpack_root && expand_modpacks.get(),
                                    index: from.index,
                                };

                                let to_key = DependencyNodeKey {
                                    view: current_view,
                                    expand_modpacks: is_modpack_root && expand_modpacks.get(),
                                    index: to.index,
                                };

                                Some(view! {
                                    <path
                                        class="deptree-edge"
                                        d=move || {
                                            let from_offset =
                                                node_offsets.with(|offsets| {
                                                    offsets
                                                        .get(&from_key)
                                                        .copied()
                                                        .unwrap_or((0.0, 0.0))
                                                });

                                            let to_offset =
                                                node_offsets.with(|offsets| {
                                                    offsets
                                                        .get(&to_key)
                                                        .copied()
                                                        .unwrap_or((0.0, 0.0))
                                                });

                                            edge_geometry(
                                                &from,
                                                from_offset,
                                                &to,
                                                to_offset,
                                            )
                                        }
                                    />
                                })
                            })
                            .collect_view()
                    }}
                </svg>

                <div class="deptree-content">
                    {move || {
                        let current_layout = layout.get();
                        let root_index = current_layout.root_index;
                        let current_view = view_mode.get();

                        current_layout
                            .nodes
                            .into_iter()
                            .map(|node| {
                                let is_target =
                                    node.index == root_index;

                                let node_key = DependencyNodeKey {
                                    view: current_view,
                                    expand_modpacks: is_modpack_root && expand_modpacks.get(),
                                    index: node.index,
                                };

                                let node_offset = move || {
                                    node_offsets.with(|offsets| {
                                        offsets
                                            .get(&node_key)
                                            .copied()
                                            .unwrap_or((0.0, 0.0))
                                    })
                                };

                                let project_ref = RegistryProjectRef {
                                    project_kind: node.project_kind,
                                    project_id: node.project_id.clone(),
                                };

                                let title = node.title.clone();
                                let description = node.description.clone();
                                let id = node.project_id.clone();

                                let kind =
                                    project_kind_label(node.project_kind);

                                let meta = match &node.version {
                                    Some(version) => {
                                        format!("{kind} · v{version}")
                                    }

                                    None if node.cycle => {
                                        format!("{kind} · cycle")
                                    }

                                    None => {
                                        format!("{kind} · not published")
                                    }
                                };

                                let available = node.available;
                                let cycle = node.cycle;

                                let left = node.x;
                                let top = node.y;

                                view! {
                                    <button
                                        type="button"
                                        class="deptree-node"
                                        class=("target", is_target)
                                        class=("missing", !available)
                                        class=("cycle", cycle)
                                        class=(
                                            "modpack",
                                            node.project_kind
                                                == RegistryProjectKind::Modpack,
                                        )
                                        class=(
                                            "dragging",
                                            move || {
                                                dragging_node.get()
                                                    == Some(node_key)
                                            },
                                        )
                                        disabled=!available

                                        on:mousedown=move |event: MouseEvent| {
                                            event.prevent_default();
                                            event.stop_propagation();

                                            set_is_dragging.set(true);
                                            dragging_node.set(Some(node_key));
                                            drag_distance.set(0.0);
                                            suppress_next_click.set(false);

                                            set_drag_start.set((
                                                event.client_x() as f64,
                                                event.client_y() as f64,
                                            ));
                                        }

                                        on:click=move |event| {
                                            if suppress_next_click.get() {
                                                event.prevent_default();
                                                suppress_next_click.set(false);
                                            } else if available {
                                                on_open_project.run(
                                                    project_ref.clone(),
                                                );
                                            }
                                        }

                                        style:left=move || {
                                            let (node_x, _) = node_offset();

                                            format!(
                                                "{}px",
                                                left
                                                    + node_x
                                                    - NODE_WIDTH * 0.5
                                            )
                                        }

                                        style:top=move || {
                                            let (_, node_y) = node_offset();

                                            format!(
                                                "{}px",
                                                top
                                                    + node_y
                                                    - NODE_HEIGHT * 0.5
                                            )
                                        }
                                    >
                                        <strong>{id}</strong>
                                        <span>{meta}</span>

                                        <Show when=move || cycle>
                                            <small>"cycle ends here"</small>
                                        </Show>

                                        <span class="deptree-tooltip" role="tooltip">
                                            <strong>{title}</strong>
                                            <span>{description}</span>
                                        </span>
                                    </button>
                                }
                            })
                            .collect_view()
                    }}
                </div>
            </div>

            <div class="deptree-hint">
                "Drag the background to pan · drag nodes to rearrange"
            </div>
        </div>
    }
}

fn view_button_class(
    current: DependencyTreeView,
    target: DependencyTreeView,
) -> &'static str {
    if current == target {
        "active"
    } else {
        ""
    }
}

fn dependency_graph_projection(
    graph: &RegistryDependencyGraph,
    expand_modpacks: bool,
) -> RegistryDependencyGraph {
    if graph.nodes.is_empty() || graph.root_index >= graph.nodes.len() {
        return RegistryDependencyGraph {
            root_index: 0,
            nodes: Vec::new(),
            edges: Vec::new(),
        };
    }

    if graph.nodes[graph.root_index].project_kind != RegistryProjectKind::Modpack {
        return graph.clone();
    }

    if expand_modpacks {
        expand_nested_modpacks(graph)
    } else {
        collapse_nested_modpacks(graph)
    }
}

fn graph_adjacency(
    graph: &RegistryDependencyGraph,
) -> Vec<Vec<RegistryDependencyGraphEdge>> {
    let mut adjacency =
        vec![Vec::<RegistryDependencyGraphEdge>::new(); graph.nodes.len()];

    for edge in &graph.edges {
        if edge.from < adjacency.len() && edge.to < adjacency.len() {
            adjacency[edge.from].push(edge.clone());
        }
    }

    adjacency
}

fn collapse_nested_modpacks(
    graph: &RegistryDependencyGraph,
) -> RegistryDependencyGraph {
    let adjacency = graph_adjacency(graph);
    let root = graph.root_index;
    let mut nodes = vec![graph.nodes[root].clone()];
    let mut indices = HashMap::from([(root, 0_usize)]);
    let mut queue = VecDeque::from([root]);
    let mut expanded = HashSet::new();
    let mut edges = Vec::new();
    let mut seen_edges = HashSet::new();

    while let Some(source) = queue.pop_front() {
        if !expanded.insert(source) {
            continue;
        }
        if source != root
            && graph.nodes[source].project_kind == RegistryProjectKind::Modpack
        {
            continue;
        }

        let from = indices[&source];
        for edge in &adjacency[source] {
            let target = edge.to;
            let to = if let Some(index) = indices.get(&target).copied() {
                index
            } else {
                let index = nodes.len();
                nodes.push(graph.nodes[target].clone());
                indices.insert(target, index);
                queue.push_back(target);
                index
            };

            if seen_edges.insert((from, to, edge.kind)) {
                edges.push(RegistryDependencyGraphEdge {
                    from,
                    to,
                    kind: edge.kind,
                });
            }
        }
    }

    RegistryDependencyGraph {
        root_index: 0,
        nodes,
        edges,
    }
}

fn expand_nested_modpacks(
    graph: &RegistryDependencyGraph,
) -> RegistryDependencyGraph {
    let adjacency = graph_adjacency(graph);
    let root = graph.root_index;
    let mut nodes = vec![graph.nodes[root].clone()];
    let mut indices = HashMap::from([(root, 0_usize)]);
    let mut queue = VecDeque::from([root]);
    let mut expanded = HashSet::new();
    let mut edges = Vec::new();
    let mut seen_edges = HashSet::new();

    while let Some(source) = queue.pop_front() {
        if !expanded.insert(source) {
            continue;
        }

        let from = indices[&source];
        for edge in &adjacency[source] {
            let mut targets = Vec::new();
            let mut hidden_path = HashSet::new();
            collect_expanded_targets(
                graph,
                &adjacency,
                root,
                edge.to,
                edge.kind,
                &mut hidden_path,
                &mut targets,
            );

            for (target, kind) in targets {
                let to = if let Some(index) = indices.get(&target).copied() {
                    index
                } else {
                    let index = nodes.len();
                    nodes.push(graph.nodes[target].clone());
                    indices.insert(target, index);
                    queue.push_back(target);
                    index
                };

                if seen_edges.insert((from, to, kind)) {
                    edges.push(RegistryDependencyGraphEdge {
                        from,
                        to,
                        kind,
                    });
                }
            }
        }
    }

    RegistryDependencyGraph {
        root_index: 0,
        nodes,
        edges,
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_expanded_targets(
    graph: &RegistryDependencyGraph,
    adjacency: &[Vec<RegistryDependencyGraphEdge>],
    root: usize,
    target: usize,
    kind: RegistryDependencyKind,
    hidden_path: &mut HashSet<usize>,
    targets: &mut Vec<(usize, RegistryDependencyKind)>,
) {
    let Some(node) = graph.nodes.get(target) else {
        return;
    };

    if target != root
        && node.project_kind == RegistryProjectKind::Modpack
        && node.available
    {
        if !hidden_path.insert(target) {
            return;
        }

        for edge in &adjacency[target] {
            collect_expanded_targets(
                graph,
                adjacency,
                root,
                edge.to,
                edge.kind,
                hidden_path,
                targets,
            );
        }

        hidden_path.remove(&target);
    } else {
        targets.push((target, kind));
    }
}

fn layout_dependency_graph(
    graph: &RegistryDependencyGraph,
) -> DependencyTreeLayout {
    if graph.nodes.is_empty() || graph.root_index >= graph.nodes.len() {
        return DependencyTreeLayout {
            nodes: Vec::new(),
            edges: Vec::new(),
            root_index: 0,
            truncated: false,
        };
    }

    let mut adjacency =
        vec![Vec::<usize>::new(); graph.nodes.len()];

    for edge in &graph.edges {
        if edge.from < adjacency.len()
            && edge.to < adjacency.len()
        {
            adjacency[edge.from].push(edge.to);
        }
    }

    let mut depths =
        vec![usize::MAX; graph.nodes.len()];

    depths[graph.root_index] = 0;

    let mut queue =
        VecDeque::from([graph.root_index]);

    while let Some(index) = queue.pop_front() {
        let next_depth =
            depths[index].saturating_add(1);

        for &target in &adjacency[index] {
            if next_depth < depths[target] {
                depths[target] = next_depth;
                queue.push_back(target);
            }
        }
    }

    let fallback_depth = depths
        .iter()
        .copied()
        .filter(|depth| *depth != usize::MAX)
        .max()
        .unwrap_or(0)
        .saturating_add(1);

    for depth in &mut depths {
        if *depth == usize::MAX {
            *depth = fallback_depth;
        }
    }

    let mut counts =
        HashMap::<usize, usize>::new();

    for depth in &depths {
        *counts.entry(*depth).or_default() += 1;
    }

    let mut positions =
        HashMap::<usize, usize>::new();

    let nodes = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let depth = depths[index];

            let position =
                positions.entry(depth).or_default();

            let count = counts[&depth];

            let x = (
                *position as f64
                    - (count.saturating_sub(1) as f64 * 0.5)
            ) * COLUMN_GAP;

            *position += 1;

            dependency_tree_node(
                index,
                node,
                false,
                x,
                depth as f64 * ROW_GAP,
            )
        })
        .collect();

    let edges = graph
        .edges
        .iter()
        .filter(|edge| {
            edge.from < graph.nodes.len()
                && edge.to < graph.nodes.len()
        })
        .map(|edge| DependencyTreeEdge {
            from: edge.from,
            to: edge.to,
            kind: edge.kind,
        })
        .collect();

    DependencyTreeLayout {
        nodes,
        edges,
        root_index: graph.root_index,
        truncated: false,
    }
}

fn layout_dependency_tree(
    graph: &RegistryDependencyGraph,
) -> DependencyTreeLayout {
    if graph.nodes.is_empty()
        || graph.root_index >= graph.nodes.len()
    {
        return DependencyTreeLayout {
            nodes: Vec::new(),
            edges: Vec::new(),
            root_index: 0,
            truncated: false,
        };
    }

    let mut adjacency =
        vec![
            Vec::<RegistryDependencyGraphEdge>::new();
            graph.nodes.len()
        ];

    for edge in &graph.edges {
        if edge.from < adjacency.len()
            && edge.to < adjacency.len()
        {
            adjacency[edge.from].push(edge.clone());
        }
    }

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut leaf_cursor = 0_usize;
    let mut path = HashSet::new();
    let mut truncated = false;
    let mut remaining = MAX_TREE_LAYOUT_NODES;

    let root_index = layout_dependency_tree_node(
        graph,
        &adjacency,
        graph.root_index,
        0,
        false,
        &mut path,
        &mut nodes,
        &mut edges,
        &mut leaf_cursor,
        &mut truncated,
        &mut remaining,
    )
    .map(|(index, _)| index)
    .unwrap_or(0);

    if let Some(root_x) =
        nodes.get(root_index).map(|node| node.x)
    {
        for node in &mut nodes {
            node.x -= root_x;
        }
    }

    DependencyTreeLayout {
        nodes,
        edges,
        root_index,
        truncated,
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_dependency_tree_node(
    graph: &RegistryDependencyGraph,
    adjacency: &[Vec<RegistryDependencyGraphEdge>],
    source_index: usize,
    depth: usize,
    cycle: bool,
    path: &mut HashSet<usize>,
    nodes: &mut Vec<DependencyTreeNode>,
    edges: &mut Vec<DependencyTreeEdge>,
    leaf_cursor: &mut usize,
    truncated: &mut bool,
    remaining: &mut usize,
) -> Option<(usize, f64)> {
    if *remaining == 0 {
        *truncated = true;
        return None;
    }

    *remaining -= 1;

    let source =
        graph.nodes.get(source_index)?;

    let mut children = Vec::new();

    if !cycle {
        path.insert(source_index);

        for edge in &adjacency[source_index] {
            let child_cycle =
                path.contains(&edge.to);

            if let Some((child_index, child_x)) =
                layout_dependency_tree_node(
                    graph,
                    adjacency,
                    edge.to,
                    depth + 1,
                    child_cycle,
                    path,
                    nodes,
                    edges,
                    leaf_cursor,
                    truncated,
                    remaining,
                )
            {
                children.push((
                    child_index,
                    child_x,
                    edge.kind,
                ));
            }

            if *remaining == 0 {
                *truncated = true;
                break;
            }
        }

        path.remove(&source_index);
    }

    let x = if children.is_empty() {
        let x =
            *leaf_cursor as f64 * COLUMN_GAP;

        *leaf_cursor += 1;

        x
    } else {
        children
            .iter()
            .map(|(_, x, _)| *x)
            .sum::<f64>()
            / children.len() as f64
    };

    let index = nodes.len();

    nodes.push(dependency_tree_node(
        index,
        source,
        cycle,
        x,
        depth as f64 * ROW_GAP,
    ));

    for (child_index, _, kind) in children {
        edges.push(DependencyTreeEdge {
            from: index,
            to: child_index,
            kind,
        });
    }

    Some((index, x))
}

fn dependency_tree_node(
    index: usize,
    node: &RegistryDependencyGraphNode,
    cycle: bool,
    x: f64,
    y: f64,
) -> DependencyTreeNode {
    DependencyTreeNode {
        index,
        project_kind: node.project_kind,
        project_id: node.project_id.clone(),
        title: node.title.clone(),
        description: node.description.clone(),
        version: node.version.clone(),
        available: node.available,
        cycle,
        x,
        y,
    }
}

fn edge_geometry(
    from: &DependencyTreeNode,
    from_offset: (f64, f64),
    to: &DependencyTreeNode,
    to_offset: (f64, f64),
) -> String {
    let from_x =
        from.x + from_offset.0;

    let from_y =
        from.y + from_offset.1 + NODE_HEIGHT * 0.5;

    let to_x =
        to.x + to_offset.0;

    let to_y =
        to.y + to_offset.1 - NODE_HEIGHT * 0.5;

    let control_y =
        (from_y + to_y) * 0.5;

    format!(
        "M {from_x} {from_y} C {from_x} {control_y}, {to_x} {control_y}, {to_x} {to_y}"
    )
}

fn project_kind_label(
    kind: RegistryProjectKind,
) -> &'static str {
    match kind {
        RegistryProjectKind::Mod => "Mod",
        RegistryProjectKind::Modpack => "Modpack",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        id: &str,
    ) -> RegistryDependencyGraphNode {
        RegistryDependencyGraphNode {
            project_kind: RegistryProjectKind::Mod,
            project_id: id.to_owned(),
            title: id.to_owned(),
            description: format!("Description for {id}"),
            version: Some("1.0.0".to_owned()),
            available: true,
        }
    }

    fn modpack_node(id: &str) -> RegistryDependencyGraphNode {
        let mut node = node(id);
        node.project_kind = RegistryProjectKind::Modpack;
        node
    }

    #[test]
    fn compact_layout_keeps_shared_nodes_unique() {
        let graph =
            RegistryDependencyGraph {
                root_index: 0,

                nodes: vec![
                    node("root"),
                    node("left"),
                    node("right"),
                    node("shared"),
                ],

                edges: vec![
                    RegistryDependencyGraphEdge {
                        from: 0,
                        to: 1,
                        kind: RegistryDependencyKind::Run,
                    },

                    RegistryDependencyGraphEdge {
                        from: 0,
                        to: 2,
                        kind: RegistryDependencyKind::Run,
                    },

                    RegistryDependencyGraphEdge {
                        from: 1,
                        to: 3,
                        kind: RegistryDependencyKind::Init,
                    },

                    RegistryDependencyGraphEdge {
                        from: 2,
                        to: 3,
                        kind: RegistryDependencyKind::Init,
                    },
                ],
            };

        let layout =
            layout_dependency_graph(&graph);

        assert_eq!(layout.nodes.len(), 4);
        assert_eq!(layout.edges.len(), 4);
        assert_eq!(layout.root_index, 0);
    }

    #[test]
    fn tree_layout_stops_cycles() {
        let graph =
            RegistryDependencyGraph {
                root_index: 0,

                nodes: vec![
                    node("root"),
                    node("child"),
                ],

                edges: vec![
                    RegistryDependencyGraphEdge {
                        from: 0,
                        to: 1,
                        kind: RegistryDependencyKind::Run,
                    },

                    RegistryDependencyGraphEdge {
                        from: 1,
                        to: 0,
                        kind: RegistryDependencyKind::Run,
                    },
                ],
            };

        let layout =
            layout_dependency_tree(&graph);

        assert_eq!(layout.nodes.len(), 3);

        assert!(
            layout.nodes.iter().any(|node| node.cycle)
        );

        assert!(!layout.truncated);
    }

    #[test]
    fn collapsed_modpack_projection_stops_at_nested_modpack() {
        let graph = RegistryDependencyGraph {
            root_index: 0,
            nodes: vec![
                modpack_node("root"),
                modpack_node("nested"),
                node("nested-mod"),
            ],
            edges: vec![
                RegistryDependencyGraphEdge {
                    from: 0,
                    to: 1,
                    kind: RegistryDependencyKind::Modpack,
                },
                RegistryDependencyGraphEdge {
                    from: 1,
                    to: 2,
                    kind: RegistryDependencyKind::Mod,
                },
            ],
        };

        let projected = dependency_graph_projection(&graph, false);

        assert_eq!(projected.nodes.len(), 2);
        assert_eq!(projected.edges.len(), 1);
        assert_eq!(projected.nodes[1].project_kind, RegistryProjectKind::Modpack);
    }

    #[test]
    fn expanded_modpack_projection_splices_nested_modpack() {
        let graph = RegistryDependencyGraph {
            root_index: 0,
            nodes: vec![
                modpack_node("root"),
                modpack_node("nested"),
                node("nested-mod"),
            ],
            edges: vec![
                RegistryDependencyGraphEdge {
                    from: 0,
                    to: 1,
                    kind: RegistryDependencyKind::Modpack,
                },
                RegistryDependencyGraphEdge {
                    from: 1,
                    to: 2,
                    kind: RegistryDependencyKind::Mod,
                },
            ],
        };

        let projected = dependency_graph_projection(&graph, true);

        assert_eq!(projected.nodes.len(), 2);
        assert_eq!(projected.edges.len(), 1);
        assert_eq!(projected.nodes[0].project_kind, RegistryProjectKind::Modpack);
        assert_eq!(projected.nodes[1].project_kind, RegistryProjectKind::Mod);
        assert_eq!(projected.nodes[1].project_id, "nested-mod");
        assert_eq!(projected.edges[0].kind, RegistryDependencyKind::Mod);
    }

}
