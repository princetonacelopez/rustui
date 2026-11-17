//! Debug inspector overlay, live runtime introspection snapshot, remote IPC server,
//! and Perfetto Chrome Trace exporter.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustui_core::{Color, CornerRadii, FrameMetrics, Rect, Runtime};

use crate::damage::DamageTracker;
use crate::display_list::DisplayList;

/// Serializable snapshot of one retained UI node for the inspector tree & remote Anvil IDE client.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorNodeSnapshot {
    pub id: String,
    pub parent: Option<String>,
    pub kind: String,
    pub label: Option<String>,
    pub bounds: Rect,
    pub hidden: bool,
    pub focused: bool,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
    pub source_col: Option<u32>,
    pub children: Vec<String>,
}

/// Complete frame snapshot emitted by [`InspectorOverlay::capture_snapshot`] and [`InspectorServer`].
#[derive(Debug, Clone)]
pub struct InspectorFrameSnapshot {
    pub root_id: Option<String>,
    pub selected_node: Option<String>,
    pub node_count: usize,
    pub state_count: usize,
    pub memo_count: usize,
    pub metrics: FrameMetrics,
    pub dirty_regions: Vec<Rect>,
    pub nodes: Vec<InspectorNodeSnapshot>,
}

impl InspectorFrameSnapshot {
    /// Serializes this snapshot as a structured JSON object (zero external deps).
    pub fn to_json_string(&self) -> String {
        let mut out = String::from("{");
        out.push_str(&format!(
            "\"node_count\":{},\"state_count\":{},\"memo_count\":{},\"frame_number\":{},\"reactive_updates\":{},\"laid_out_nodes\":{},\"layout_cache_hits\":{},\"layout_cache_misses\":{},\"paint_commands\":{},\"nodes\":[",
            self.node_count,
            self.state_count,
            self.memo_count,
            self.metrics.frame_number,
            self.metrics.reactive_updates,
            self.metrics.laid_out_nodes,
            self.metrics.layout_cache_hits,
            self.metrics.layout_cache_misses,
            self.metrics.paint_commands,
        ));
        for (idx, n) in self.nodes.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            let file_json = n
                .source_file
                .as_deref()
                .map(|s| format!("\"{}\"", escape_json(s)))
                .unwrap_or_else(|| "null".into());
            let label_json = n
                .label
                .as_deref()
                .map(|s| format!("\"{}\"", escape_json(s)))
                .unwrap_or_else(|| "null".into());
            out.push_str(&format!(
                "{{\"id\":\"{}\",\"kind\":\"{}\",\"label\":{},\"bounds\":[{:.1},{:.1},{:.1},{:.1}],\"hidden\":{},\"focused\":{},\"source_file\":{},\"source_line\":{},\"source_col\":{}}}",
                escape_json(&n.id),
                escape_json(&n.kind),
                label_json,
                n.bounds.x,
                n.bounds.y,
                n.bounds.width,
                n.bounds.height,
                n.hidden,
                n.focused,
                file_json,
                n.source_line.map_or_else(|| "null".into(), |l| l.to_string()),
                n.source_col.map_or_else(|| "null".into(), |c| c.to_string()),
            ));
        }
        out.push_str("]}");
        out
    }
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Exports frame timing phases in Chrome/Perfetto JSON trace event format.
pub fn export_perfetto_trace(metrics: &FrameMetrics) -> String {
    let sig_us = metrics.signal_update_time.as_secs_f64() * 1_000_000.0;
    let lay_us = metrics.layout_time.as_secs_f64() * 1_000_000.0;
    let pnt_us = metrics.paint_time.as_secs_f64() * 1_000_000.0;
    format!(
        "{{\"traceEvents\":[\
        {{\"name\":\"signal_flush\",\"cat\":\"rustui\",\"ph\":\"X\",\"ts\":0,\"dur\":{sig_us:.1},\"pid\":1,\"tid\":1}},\
        {{\"name\":\"layout\",\"cat\":\"rustui\",\"ph\":\"X\",\"ts\":{sig_us:.1},\"dur\":{lay_us:.1},\"pid\":1,\"tid\":1}},\
        {{\"name\":\"paint\",\"cat\":\"rustui\",\"ph\":\"X\",\"ts\":{ts2:.1},\"dur\":{pnt_us:.1},\"pid\":1,\"tid\":1}}\
        ]}}",
        ts2 = sig_us + lay_us
    )
}

/// Local non-blocking TCP inspector server allowing external tools such as Anvil IDE to inspect
/// a running RustUI application's live node tree, layout bounds, and frame timings.
pub struct InspectorServer {
    listener: TcpListener,
    addr: SocketAddr,
    latest_json: Arc<Mutex<String>>,
}

impl std::fmt::Debug for InspectorServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectorServer")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl InspectorServer {
    /// Binds to `127.0.0.1:<port>` (pass `0` for an ephemeral port) in non-blocking mode.
    pub fn bind(port: u16) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("failed to bind inspector server: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("failed to configure inspector socket: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("failed to read inspector address: {e}"))?;
        Ok(Self {
            listener,
            addr,
            latest_json: Arc::new(Mutex::new("{\"nodes\":[]}".into())),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Updates the published snapshot and serves any pending local client requests without blocking.
    pub fn publish_and_poll(&self, snapshot: &InspectorFrameSnapshot) {
        let json = snapshot.to_json_string();
        if let Ok(mut guard) = self.latest_json.lock() {
            *guard = json.clone();
        }
        while let Ok((mut stream, _)) = self.listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
            let mut req_buf = [0u8; 64];
            let _ = stream.read(&mut req_buf);
            let _ = stream.write_all(json.as_bytes());
            let _ = stream.write_all(b"\n");
        }
    }
}

/// Renders node layout boxes, damaged dirty regions, and signal metrics on top of a frame.
#[derive(Debug, Default, Clone, Copy)]
pub struct InspectorOverlay {
    pub enabled: bool,
    pub show_bounds: bool,
    pub show_dirty_regions: bool,
    pub show_stats_badge: bool,
    pub selected_highlight: Option<Rect>,
}

impl InspectorOverlay {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            show_bounds: true,
            show_dirty_regions: true,
            show_stats_badge: true,
            selected_highlight: None,
        }
    }

    /// Captures a structured [`InspectorFrameSnapshot`] from `runtime` and `damage`.
    pub fn capture_snapshot(
        &self,
        runtime: &Runtime,
        damage: &DamageTracker,
    ) -> InspectorFrameSnapshot {
        runtime.with_inner(|inner| {
            let mut nodes = Vec::with_capacity(inner.nodes.len());
            for (id, node) in &inner.nodes {
                let (sf, sl, sc) = match node.source_location {
                    Some((f, l, c)) => (Some(f.to_owned()), Some(l), Some(c)),
                    None => (None, None, None),
                };
                nodes.push(InspectorNodeSnapshot {
                    id: format!("{id:?}"),
                    parent: node.parent.map(|p| format!("{p:?}")),
                    kind: format!("{:?}", node.kind)
                        .split(['{', '('])
                        .next()
                        .unwrap_or("Node")
                        .trim()
                        .to_owned(),
                    label: node.accessibility.label.clone(),
                    bounds: node.bounds(),
                    hidden: node.visual_props.hidden,
                    focused: node.focus.focused,
                    source_file: sf,
                    source_line: sl,
                    source_col: sc,
                    children: node.children.iter().map(|c| format!("{c:?}")).collect(),
                });
            }
            InspectorFrameSnapshot {
                root_id: inner.root.map(|r| format!("{r:?}")),
                selected_node: None,
                node_count: inner.nodes.len(),
                state_count: runtime.state_count(),
                memo_count: runtime.memo_count(),
                metrics: inner.scheduler.metrics,
                dirty_regions: damage.last_regions().to_vec(),
                nodes,
            }
        })
    }

    pub fn emit(
        &self,
        runtime: &Runtime,
        damage: &DamageTracker,
        list: &mut DisplayList,
        viewport: Rect,
    ) {
        if !self.enabled {
            return;
        }

        if self.show_bounds {
            runtime.with_inner(|inner| {
                for (_, node) in &inner.nodes {
                    let b = node.bounds();
                    if b.width <= 0.0 || b.height <= 0.0 {
                        continue;
                    }
                    list.stroke_rounded_rect(
                        b,
                        CornerRadii::ZERO,
                        1.0,
                        Color::rgba(0.0, 0.85, 0.90, 0.55),
                    );
                }
            });
        }

        if let Some(sel) = self.selected_highlight {
            list.fill_rounded_rect(sel, CornerRadii::ZERO, Color::rgba(0.25, 0.62, 1.0, 0.18));
            list.stroke_rounded_rect(
                sel,
                CornerRadii::ZERO,
                2.0,
                Color::rgba(0.25, 0.62, 1.0, 0.95),
            );
        }

        if self.show_dirty_regions {
            for region in damage.last_regions() {
                list.fill_rounded_rect(
                    *region,
                    CornerRadii::ZERO,
                    Color::rgba(0.96, 0.16, 0.58, 0.14),
                );
                list.stroke_rounded_rect(
                    *region,
                    CornerRadii::ZERO,
                    1.5,
                    Color::rgba(0.96, 0.16, 0.58, 0.85),
                );
            }
        }

        if self.show_stats_badge {
            let badge_rect = Rect::new((viewport.max_x() - 220.0).max(8.0), 8.0, 212.0, 52.0);
            list.fill_rounded_rect(
                badge_rect,
                CornerRadii::all(6.0),
                Color::rgba(0.06, 0.08, 0.12, 0.88),
            );
            list.stroke_rounded_rect(
                badge_rect,
                CornerRadii::all(6.0),
                1.0,
                Color::rgba(0.35, 0.72, 1.0, 0.85),
            );
        }
    }
}
