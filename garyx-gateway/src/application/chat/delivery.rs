use std::sync::Arc;

mod plan;

use self::plan::{
    BoundThreadDeliveryTarget, snapshot_bound_thread_delivery_targets,
    targets_except_streaming_target,
};
use garyx_channels::{StreamingDispatchTarget, build_outbound_stream_callback};
use garyx_models::provider::StreamEvent;

use crate::server::AppState;

pub async fn build_bound_response_callback(
    state: &Arc<AppState>,
    thread_id: &str,
    run_id: &str,
    streaming_target: Option<StreamingDispatchTarget>,
) -> Result<
    Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    garyx_channels::committed_replay::CommittedReplayError,
> {
    let targets = snapshot_bound_thread_delivery_targets(state, thread_id).await;
    if let Some(target) = streaming_target.as_ref()
        && let Some(callback) = state
            .channel_dispatcher()
            .build_streaming_callback(target.clone(), state.threads.router.clone())
    {
        let bound_consumer = build_bound_delivery_consumer(
            state.clone(),
            thread_id.to_owned(),
            targets_except_streaming_target(&targets, target),
        );

        let consumer = if let Some(bound_consumer) = bound_consumer {
            Arc::new(move |event: StreamEvent| {
                callback(event.clone());
                bound_consumer(event);
            }) as Arc<dyn Fn(StreamEvent) + Send + Sync>
        } else {
            callback
        };
        // Read this run's stream from the durable committed transcript. The
        // streaming sender is unchanged; only the source changes.
        return garyx_channels::committed_replay::committed_callback(
            &state.integration.bridge,
            run_id,
            consumer,
        )
        .await;
    }

    let Some(bound_consumer) =
        build_bound_delivery_consumer(state.clone(), thread_id.to_owned(), targets)
    else {
        return Ok(None);
    };

    // Read this run's stream from the durable committed transcript. The bound
    // delivery buffer is unchanged; only the source changes.
    garyx_channels::committed_replay::committed_callback(
        &state.integration.bridge,
        run_id,
        bound_consumer,
    )
    .await
}

fn delivery_target_to_streaming_target(
    target_thread_id: &str,
    target: BoundThreadDeliveryTarget,
) -> StreamingDispatchTarget {
    StreamingDispatchTarget {
        target_thread_id: target_thread_id.to_owned(),
        channel: target.channel,
        account_id: target.account_id,
        chat_id: target.chat_id,
        delivery_target_type: target.delivery_target_type,
        delivery_target_id: target.delivery_target_id,
        thread_id: target.thread_id,
    }
}

fn build_bound_delivery_consumer(
    state: Arc<AppState>,
    thread_id: String,
    targets: Vec<BoundThreadDeliveryTarget>,
) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
    if targets.is_empty() {
        return None;
    }

    let router = state.threads.router.clone();
    let dispatcher = state.channel_dispatcher();
    let callbacks: Vec<Arc<dyn Fn(StreamEvent) + Send + Sync>> = targets
        .into_iter()
        .map(|target| delivery_target_to_streaming_target(&thread_id, target))
        .map(|target| {
            dispatcher
                .build_streaming_callback(target.clone(), router.clone())
                .unwrap_or_else(|| {
                    build_outbound_stream_callback(dispatcher.clone(), target, router.clone())
                })
        })
        .collect();

    if callbacks.is_empty() {
        return None;
    }

    let bound_consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        for callback in &callbacks {
            callback(event.clone());
        }
    });
    Some(bound_consumer)
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
