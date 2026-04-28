use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;

/// Boxed async future returned by graph node handlers.
pub type NodeFuture<'a, N, O, E> =
    Pin<Box<dyn Future<Output = Result<GraphTransition<N, O>, E>> + Send + 'a>>;

/// Handler signature for one graph node.
pub type NodeHandler<S, N, O, E> = for<'a> fn(&'a mut S) -> NodeFuture<'a, N, O, E>;

/// Result of a node execution: continue to next node or terminate graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphTransition<N, O> {
    Next(N),
    End(O),
}

/// Build-time validation errors for graph topology registration.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GraphBuildError<N> {
    #[error("duplicate graph node: {0:?}")]
    DuplicateNode(N),
    #[error("graph entry node is not registered: {0:?}")]
    MissingEntryNode(N),
    #[error("graph has no registered nodes")]
    EmptyGraph,
}

/// Runtime errors produced by graph execution.
#[derive(Debug, thiserror::Error)]
pub enum GraphError<N, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    #[error("graph node not found: {0:?}")]
    NodeNotFound(N),
    #[error("graph exceeded max steps ({0})")]
    StepLimitExceeded(usize),
    #[error(transparent)]
    Node(#[from] E),
}

/// Execution result with output and run metadata.
#[derive(Debug)]
pub struct GraphRunResult<N, O> {
    pub output: O,
    pub steps: usize,
    pub visited: Vec<N>,
}

/// Minimal async graph runtime inspired by pydantic-graph semantics.
pub struct Graph<S, N, O, E>
where
    N: Copy + Eq + Hash,
    E: std::error::Error + Send + Sync + 'static,
{
    entry_node: N,
    max_steps: usize,
    nodes: HashMap<N, NodeHandler<S, N, O, E>>,
}

impl<S, N, O, E> Graph<S, N, O, E>
where
    N: Copy + Eq + Hash,
    E: std::error::Error + Send + Sync + 'static,
{
    pub fn new(entry_node: N) -> Self {
        Self {
            entry_node,
            max_steps: 256,
            nodes: HashMap::new(),
        }
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }

    pub fn add_node(
        &mut self,
        name: N,
        handler: NodeHandler<S, N, O, E>,
    ) -> Result<(), GraphBuildError<N>> {
        if self.nodes.contains_key(&name) {
            return Err(GraphBuildError::DuplicateNode(name));
        }
        self.nodes.insert(name, handler);
        Ok(())
    }

    pub fn validate(&self) -> Result<(), GraphBuildError<N>> {
        if self.nodes.is_empty() {
            return Err(GraphBuildError::EmptyGraph);
        }
        if !self.nodes.contains_key(&self.entry_node) {
            return Err(GraphBuildError::MissingEntryNode(self.entry_node));
        }
        Ok(())
    }

    pub async fn run(&self, state: &mut S) -> Result<GraphRunResult<N, O>, GraphError<N, E>> {
        self.validate().map_err(|err| match err {
            GraphBuildError::EmptyGraph => GraphError::NodeNotFound(self.entry_node),
            GraphBuildError::MissingEntryNode(node) => GraphError::NodeNotFound(node),
            GraphBuildError::DuplicateNode(_) => unreachable!("graph validated at build-time"),
        })?;

        let mut current = self.entry_node;
        let mut visited = Vec::new();
        let mut steps = 0usize;

        loop {
            if steps >= self.max_steps {
                return Err(GraphError::StepLimitExceeded(self.max_steps));
            }
            let handler = self
                .nodes
                .get(&current)
                .ok_or(GraphError::NodeNotFound(current))?;
            visited.push(current);
            steps += 1;
            match handler(state).await? {
                GraphTransition::Next(next) => current = next,
                GraphTransition::End(output) => {
                    return Ok(GraphRunResult {
                        output,
                        steps,
                        visited,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
