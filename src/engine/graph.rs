use std::collections::HashSet;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct NodeId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeKind {
    SineOscillator,
    OutputEnvelope,
    AudioOutput,
}

#[derive(Clone, Copy, Debug)]
struct GraphNode {
    id: NodeId,
    kind: NodeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Port {
    AudioOutput,
    AudioInput,
}

#[derive(Clone, Copy, Debug)]
struct PortAddress {
    node: NodeId,
    port: Port,
}

#[derive(Clone, Copy, Debug)]
struct Connection {
    from: PortAddress,
    to: PortAddress,
}

#[derive(Debug)]
pub(super) struct GraphDocument {
    nodes: Vec<GraphNode>,
    connections: Vec<Connection>,
}

#[derive(Debug)]
pub(super) struct CompiledGraph {
    oscillator: NodeId,
    envelope: NodeId,
    output: NodeId,
    execution_order: Vec<NodeId>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum GraphCompileError {
    DuplicateNodeId(NodeId),
    MissingNode(NodeId),
    InvalidPort { node: NodeId, port: Port },
    InvalidInitialTopology,
    Cycle,
}

impl fmt::Display for GraphCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNodeId(id) => write!(formatter, "duplicate graph node id {}", id.0),
            Self::MissingNode(id) => {
                write!(formatter, "connection references missing node {}", id.0)
            }
            Self::InvalidPort { node, port } => {
                write!(formatter, "invalid port {port:?} on node {}", node.0)
            }
            Self::InvalidInitialTopology => formatter.write_str(
                "the built-in graph must route one sine oscillator through one output envelope to one audio output",
            ),
            Self::Cycle => formatter.write_str("audio graph contains a cycle"),
        }
    }
}

impl std::error::Error for GraphCompileError {}

impl GraphDocument {
    pub(super) fn initial() -> Self {
        let oscillator = NodeId(1);
        let envelope = NodeId(2);
        let output = NodeId(3);

        Self {
            nodes: vec![
                GraphNode {
                    id: oscillator,
                    kind: NodeKind::SineOscillator,
                },
                GraphNode {
                    id: envelope,
                    kind: NodeKind::OutputEnvelope,
                },
                GraphNode {
                    id: output,
                    kind: NodeKind::AudioOutput,
                },
            ],
            connections: vec![
                Connection {
                    from: PortAddress {
                        node: oscillator,
                        port: Port::AudioOutput,
                    },
                    to: PortAddress {
                        node: envelope,
                        port: Port::AudioInput,
                    },
                },
                Connection {
                    from: PortAddress {
                        node: envelope,
                        port: Port::AudioOutput,
                    },
                    to: PortAddress {
                        node: output,
                        port: Port::AudioInput,
                    },
                },
            ],
        }
    }

    pub(super) fn compile(&self) -> Result<CompiledGraph, GraphCompileError> {
        let mut node_ids = HashSet::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if !node_ids.insert(node.id) {
                return Err(GraphCompileError::DuplicateNodeId(node.id));
            }
        }

        let oscillators: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::SineOscillator)
            .collect();
        let outputs: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::AudioOutput)
            .collect();
        let envelopes: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::OutputEnvelope)
            .collect();
        if oscillators.len() != 1
            || envelopes.len() != 1
            || outputs.len() != 1
            || self.nodes.len() != 3
        {
            return Err(GraphCompileError::InvalidInitialTopology);
        }

        for connection in &self.connections {
            let source = self
                .nodes
                .iter()
                .find(|node| node.id == connection.from.node)
                .ok_or(GraphCompileError::MissingNode(connection.from.node))?;
            let destination = self
                .nodes
                .iter()
                .find(|node| node.id == connection.to.node)
                .ok_or(GraphCompileError::MissingNode(connection.to.node))?;

            let valid_source = matches!(
                (source.kind, connection.from.port),
                (NodeKind::SineOscillator, Port::AudioOutput)
                    | (NodeKind::OutputEnvelope, Port::AudioOutput)
            );
            let valid_destination = matches!(
                (destination.kind, connection.to.port),
                (NodeKind::OutputEnvelope, Port::AudioInput)
                    | (NodeKind::AudioOutput, Port::AudioInput)
            );
            if !valid_source {
                return Err(GraphCompileError::InvalidPort {
                    node: source.id,
                    port: connection.from.port,
                });
            }
            if !valid_destination {
                return Err(GraphCompileError::InvalidPort {
                    node: destination.id,
                    port: connection.to.port,
                });
            }
        }

        if self.connections.len() != 2
            || self.connections[0].from.node != oscillators[0].id
            || self.connections[0].to.node != envelopes[0].id
            || self.connections[1].from.node != envelopes[0].id
            || self.connections[1].to.node != outputs[0].id
        {
            return Err(GraphCompileError::InvalidInitialTopology);
        }

        let execution_order = self.topological_order()?;
        Ok(CompiledGraph {
            oscillator: oscillators[0].id,
            envelope: envelopes[0].id,
            output: outputs[0].id,
            execution_order,
        })
    }

    fn topological_order(&self) -> Result<Vec<NodeId>, GraphCompileError> {
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut remaining: HashSet<_> = self.nodes.iter().map(|node| node.id).collect();

        while !remaining.is_empty() {
            let next = remaining
                .iter()
                .copied()
                .find(|candidate| {
                    !self.connections.iter().any(|connection| {
                        connection.to.node == *candidate
                            && remaining.contains(&connection.from.node)
                    })
                })
                .ok_or(GraphCompileError::Cycle)?;
            order.push(next);
            remaining.remove(&next);
        }

        Ok(order)
    }
}

impl CompiledGraph {
    pub(super) fn oscillator(&self) -> NodeId {
        self.oscillator
    }

    pub(super) fn output(&self) -> NodeId {
        self.output
    }

    pub(super) fn envelope(&self) -> NodeId {
        self.envelope
    }

    pub(super) fn execution_order(&self) -> &[NodeId] {
        &self.execution_order
    }
}

#[cfg(test)]
mod tests {
    use super::{GraphCompileError, GraphDocument};

    #[test]
    fn built_in_graph_compiles_oscillator_before_output() {
        let graph = GraphDocument::initial().compile().unwrap();
        assert_eq!(
            graph.execution_order(),
            &[graph.oscillator(), graph.envelope(), graph.output()]
        );
    }

    #[test]
    fn rejects_duplicate_node_ids() {
        let mut document = GraphDocument::initial();
        document.nodes[2].id = document.nodes[0].id;
        assert!(matches!(
            document.compile(),
            Err(GraphCompileError::DuplicateNodeId(_))
        ));
    }

    #[test]
    fn rejects_wrong_port_types() {
        let mut document = GraphDocument::initial();
        document.connections[0].to.port = super::Port::AudioOutput;
        assert!(matches!(
            document.compile(),
            Err(GraphCompileError::InvalidPort { .. })
        ));
    }
}
