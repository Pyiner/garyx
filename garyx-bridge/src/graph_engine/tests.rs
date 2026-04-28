use super::*;

#[derive(Debug, thiserror::Error)]
enum TestError {
    #[error("node failure")]
    NodeFailure,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
enum TestNode {
    Start,
    Second,
    Loop,
    Failing,
    Missing,
}

#[derive(Default)]
struct TestState {
    value: i32,
}

fn node_start(state: &mut TestState) -> NodeFuture<'_, TestNode, i32, TestError> {
    Box::pin(async move {
        state.value += 1;
        Ok(GraphTransition::Next(TestNode::Second))
    })
}

fn node_second(state: &mut TestState) -> NodeFuture<'_, TestNode, i32, TestError> {
    Box::pin(async move {
        state.value += 2;
        Ok(GraphTransition::End(state.value))
    })
}

fn node_loop(_state: &mut TestState) -> NodeFuture<'_, TestNode, i32, TestError> {
    Box::pin(async move { Ok(GraphTransition::Next(TestNode::Loop)) })
}

fn node_failing(_state: &mut TestState) -> NodeFuture<'_, TestNode, i32, TestError> {
    Box::pin(async move { Err(TestError::NodeFailure) })
}

#[tokio::test]
async fn linear_graph_runs_to_completion() {
    let mut graph = Graph::new(TestNode::Start);
    graph.add_node(TestNode::Start, node_start).unwrap();
    graph.add_node(TestNode::Second, node_second).unwrap();
    let mut state = TestState::default();
    let result = graph.run(&mut state).await.unwrap();
    assert_eq!(result.output, 3);
    assert_eq!(result.steps, 2);
    assert_eq!(result.visited, vec![TestNode::Start, TestNode::Second]);
}

#[tokio::test]
async fn duplicate_node_rejected() {
    let mut graph: Graph<TestState, TestNode, i32, TestError> = Graph::new(TestNode::Start);
    graph.add_node(TestNode::Start, node_start).unwrap();
    let err = graph.add_node(TestNode::Start, node_second).unwrap_err();
    assert_eq!(err, GraphBuildError::DuplicateNode(TestNode::Start));
}

#[test]
fn missing_entry_rejected_during_validation() {
    let mut graph: Graph<TestState, TestNode, i32, TestError> = Graph::new(TestNode::Missing);
    graph.add_node(TestNode::Start, node_start).unwrap();
    let err = graph.validate().unwrap_err();
    assert_eq!(err, GraphBuildError::MissingEntryNode(TestNode::Missing));
}

#[test]
fn empty_graph_rejected_during_validation() {
    let graph: Graph<TestState, TestNode, i32, TestError> = Graph::new(TestNode::Start);
    let err = graph.validate().unwrap_err();
    assert_eq!(err, GraphBuildError::EmptyGraph);
}

#[tokio::test]
async fn step_limit_stops_infinite_loop() {
    let mut graph = Graph::new(TestNode::Loop).with_max_steps(3);
    graph.add_node(TestNode::Loop, node_loop).unwrap();
    let mut state = TestState::default();
    let err = graph.run(&mut state).await.unwrap_err();
    match err {
        GraphError::StepLimitExceeded(limit) => assert_eq!(limit, 3),
        _ => panic!("unexpected error"),
    }
}

#[tokio::test]
async fn node_error_is_propagated() {
    let mut graph = Graph::new(TestNode::Failing);
    graph.add_node(TestNode::Failing, node_failing).unwrap();
    let mut state = TestState::default();
    let err = graph.run(&mut state).await.unwrap_err();
    match err {
        GraphError::Node(TestError::NodeFailure) => {}
        _ => panic!("unexpected error"),
    }
}
