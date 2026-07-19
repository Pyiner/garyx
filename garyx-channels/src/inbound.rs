//! Shared inbound dispatch pipeline for channel runtimes.
//!
//! Every built-in runtime used to hand-copy the same sequence between message
//! parsing and reply sending: build the deferred bound-endpoint fanout,
//! subscribe to the run's committed transcript, pre-fetch the thread store for
//! the deferred delegate, record an optional inbound ledger event, call
//! `route_and_dispatch`, attach the resolved thread, and settle the replay
//! subscription. This module owns that sequence once; channel-specific text
//! assembly, response callbacks, ledger payloads, and reply delivery stay with
//! each runtime.

use std::future::Future;
use std::sync::Arc;

use garyx_bridge::MultiProviderBridge;
use garyx_models::MessageLedgerEvent;
use garyx_models::provider::StreamEvent;
use garyx_router::{InboundRequest, InboundResult, MessageRouter, endpoint_key};
use tokio::sync::Mutex;

use crate::bound_fanout::{DeferredBoundStreamFanout, DeferredFanoutAgentDispatcher};
use crate::committed_replay::{CommittedReplayError, committed_callback};
use crate::dispatcher::ChannelDispatcher;

/// Why a shared inbound dispatch did not produce a routed result. The two
/// variants deliberately stay distinct: a missing committed-replay bus is an
/// internal wiring failure the runtimes only log, while a dispatch error is
/// surfaced to the end user as an error reply.
pub enum InboundDispatchFailure {
    /// Subscribing to the run's committed transcript failed; nothing was
    /// routed and no user-visible reply should be sent.
    CommittedReplay(CommittedReplayError),
    /// `route_and_dispatch` failed after the subscription was established;
    /// the subscription has been aborted.
    Dispatch(String),
}

/// Shared dependencies of the inbound dispatch sequence.
pub struct InboundPipeline<'a> {
    pub router: &'a Arc<Mutex<MessageRouter>>,
    pub bridge: &'a Arc<MultiProviderBridge>,
    pub dispatcher: &'a Arc<dyn ChannelDispatcher>,
}

impl InboundPipeline<'_> {
    /// Routes one inbound request through the shared dispatch sequence.
    ///
    /// * `response_callback` is the channel's origin stream callback; it is
    ///   wrapped by the deferred bound-endpoint fanout.
    /// * `ledger_event` is recorded inside the same router critical section
    ///   that performs the dispatch (Telegram/Feishu pass `Some`; channels
    ///   without an inbound ledger pass `None`).
    /// * `on_thread_resolved` runs right after the origin fanout attaches the
    ///   canonical thread id, before the replay subscription settles; runtimes
    ///   use it to hand the thread id to their streaming callbacks. The hook
    ///   is async so callers with deferred origin streams (the subprocess
    ///   plugin host's native stream) can attach them at the same point; sync
    ///   callers wrap their body in `async move {}`.
    pub async fn dispatch<F, Fut>(
        &self,
        request: InboundRequest,
        response_callback: Arc<dyn Fn(StreamEvent) + Send + Sync>,
        ledger_event: Option<MessageLedgerEvent>,
        on_thread_resolved: F,
    ) -> Result<InboundResult, InboundDispatchFailure>
    where
        F: FnOnce(String) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        let origin_endpoint_identity = endpoint_key(
            &request.channel,
            &request.account_id,
            &request.thread_binding_key,
        );
        let deferred_fanout = DeferredBoundStreamFanout::new(
            self.router.clone(),
            self.dispatcher.clone(),
            request.run_id.clone(),
            origin_endpoint_identity,
        );
        let fanout_consumer = deferred_fanout.consumer(response_callback);

        // Read this run's stream from the durable committed transcript:
        // subscribe before dispatch so the run's first committed record is
        // never missed. Bound non-origin endpoints attach after
        // route_and_dispatch resolves the canonical thread id.
        let replay_subscription =
            match committed_callback(self.bridge, &request.run_id, fanout_consumer).await {
                Ok(subscription) => subscription,
                Err(error) => return Err(InboundDispatchFailure::CommittedReplay(error)),
            };

        // Pre-fetch the thread store in its own short lock scope so the
        // deferred delegate can attach bound endpoints before provider
        // dispatch without widening the dispatch critical section.
        let thread_store = {
            let router = self.router.lock().await;
            router.thread_store()
        };
        let dispatch_delegate = DeferredFanoutAgentDispatcher::new(
            self.bridge.as_ref(),
            deferred_fanout.clone(),
            thread_store,
        );
        let dispatch_callback = replay_subscription.callback();

        let dispatch_result = {
            let mut router = self.router.lock().await;
            if let Some(event) = ledger_event {
                router.record_message_ledger_event(event).await;
            }
            router
                .route_and_dispatch(request, &dispatch_delegate, dispatch_callback)
                .await
        };
        match dispatch_result {
            Ok(result) => {
                // Local-command and local-reply paths skip the deferred
                // delegate, so this attach is not redundant with the one the
                // delegate performs before provider dispatch; it is idempotent
                // when both run.
                deferred_fanout.attach_thread(&result.thread_id).await;
                on_thread_resolved(result.thread_id.clone()).await;
                if result.local_reply.is_some() {
                    replay_subscription.abort();
                } else {
                    replay_subscription.detach();
                }
                Ok(result)
            }
            Err(error) => {
                replay_subscription.abort();
                Err(InboundDispatchFailure::Dispatch(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    use crate::dispatcher::{ChannelInfo, OutboundMessage, SendMessageResult};
    use crate::test_helpers::{
        ConfigurableTestProvider, attach_test_bridge_runtime, make_bridge_with, make_router,
        wait_for_provider_calls,
    };
    use garyx_models::MessageLifecycleStatus;
    use garyx_router::MessageLedgerStore;

    #[derive(Default)]
    struct NoopDispatcher;

    #[async_trait]
    impl ChannelDispatcher for NoopDispatcher {
        async fn send_message(
            &self,
            _request: OutboundMessage,
        ) -> Result<SendMessageResult, crate::channel_trait::ChannelError> {
            unreachable!("inbound pipeline tests never send outbound messages")
        }

        fn available_channels(&self) -> Vec<ChannelInfo> {
            Vec::new()
        }

        fn build_stream_event_callback(
            &self,
            _target: crate::dispatcher::StreamingDispatchTarget,
        ) -> Option<crate::dispatcher::StreamDispatchCallback> {
            None
        }
    }

    fn test_request(run_id: &str) -> InboundRequest {
        InboundRequest {
            channel: "telegram".to_owned(),
            account_id: "acct".to_owned(),
            from_id: "user-1".to_owned(),
            is_group: false,
            thread_binding_key: "user-1".to_owned(),
            message: "hello pipeline".to_owned(),
            run_id: run_id.to_owned(),
            images: Vec::new(),
            extra_metadata: HashMap::new(),
            file_paths: Vec::new(),
        }
    }

    fn ledger_event(request: &InboundRequest) -> MessageLedgerEvent {
        MessageLedgerEvent {
            ledger_id: format!("test:{}:{}", request.account_id, request.run_id),
            bot_id: format!("telegram:{}", request.account_id),
            status: MessageLifecycleStatus::Received,
            created_at: chrono::Utc::now().to_rfc3339(),
            thread_id: Some(request.thread_binding_key.clone()),
            run_id: Some(request.run_id.clone()),
            channel: Some(request.channel.clone()),
            account_id: Some(request.account_id.clone()),
            chat_id: Some(request.from_id.clone()),
            from_id: Some(request.from_id.clone()),
            native_message_id: Some("native-1".to_owned()),
            text_excerpt: Some(request.message.chars().take(200).collect()),
            terminal_reason: None,
            reply_message_id: None,
            metadata: serde_json::json!({ "source": "inbound_pipeline_test" }),
        }
    }

    #[tokio::test]
    async fn missing_committed_bus_fails_closed_before_routing() {
        let router = make_router();
        let bridge = Arc::new(MultiProviderBridge::new());
        let dispatcher: Arc<dyn ChannelDispatcher> = Arc::new(NoopDispatcher);
        let pipeline = InboundPipeline {
            router: &router,
            bridge: &bridge,
            dispatcher: &dispatcher,
        };

        let resolved = Arc::new(StdMutex::new(Vec::<String>::new()));
        let resolved_sink = resolved.clone();
        let outcome = pipeline
            .dispatch(
                test_request("run-no-bus"),
                Arc::new(|_event: StreamEvent| {}),
                None,
                move |thread_id| async move {
                    resolved_sink.lock().expect("resolved lock").push(thread_id);
                },
            )
            .await;

        assert!(matches!(
            outcome,
            Err(InboundDispatchFailure::CommittedReplay(
                CommittedReplayError::MissingEventBus
            ))
        ));
        assert!(
            resolved.lock().expect("resolved lock").is_empty(),
            "a failed subscription must not resolve a thread"
        );
    }

    #[tokio::test]
    async fn dispatch_records_ledger_and_resolves_thread() {
        let router = make_router();
        let ledger = Arc::new(MessageLedgerStore::memory());
        {
            let mut guard = router.lock().await;
            guard.set_message_ledger_store(ledger.clone());
        }
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let dispatcher: Arc<dyn ChannelDispatcher> = Arc::new(NoopDispatcher);
        let pipeline = InboundPipeline {
            router: &router,
            bridge: &bridge,
            dispatcher: &dispatcher,
        };

        let request = test_request("run-ledger");
        let event = ledger_event(&request);
        let bot_id = event.bot_id.clone();
        let resolved = Arc::new(StdMutex::new(Vec::<String>::new()));
        let resolved_sink = resolved.clone();
        let result = pipeline
            .dispatch(
                request,
                Arc::new(|_event: StreamEvent| {}),
                Some(event),
                move |thread_id| async move {
                    resolved_sink.lock().expect("resolved lock").push(thread_id);
                },
            )
            .await;

        let result = match result {
            Ok(result) => result,
            Err(InboundDispatchFailure::CommittedReplay(error)) => {
                panic!("committed subscription failed: {error}")
            }
            Err(InboundDispatchFailure::Dispatch(error)) => panic!("dispatch failed: {error}"),
        };
        assert!(result.local_reply.is_none());
        assert_eq!(
            resolved.lock().expect("resolved lock").as_slice(),
            &[result.thread_id.clone()],
            "on_thread_resolved must observe the routed thread id exactly once"
        );
        wait_for_provider_calls(&provider, 1).await;
        let events = ledger
            .list_events_for_bot(&bot_id, 50)
            .await
            .expect("ledger events");
        let inbound_events = events
            .iter()
            .filter(|event| event.ledger_id.starts_with("test:"))
            .count();
        assert_eq!(inbound_events, 1, "Some(ledger_event) records exactly once");
    }

    #[tokio::test]
    async fn dispatch_without_ledger_event_records_nothing() {
        let router = make_router();
        let ledger = Arc::new(MessageLedgerStore::memory());
        {
            let mut guard = router.lock().await;
            guard.set_message_ledger_store(ledger.clone());
        }
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let dispatcher: Arc<dyn ChannelDispatcher> = Arc::new(NoopDispatcher);
        let pipeline = InboundPipeline {
            router: &router,
            bridge: &bridge,
            dispatcher: &dispatcher,
        };

        let request = test_request("run-no-ledger");
        let bot_id = "telegram:acct".to_owned();
        let result = pipeline
            .dispatch(
                request,
                Arc::new(|_event: StreamEvent| {}),
                None,
                |_thread_id| async {},
            )
            .await;
        assert!(matches!(result, Ok(_)));
        wait_for_provider_calls(&provider, 1).await;
        let events = ledger
            .list_events_for_bot(&bot_id, 50)
            .await
            .expect("ledger events");
        assert!(
            events
                .iter()
                .all(|event| !event.ledger_id.starts_with("test:")),
            "channels without an inbound ledger must not gain one"
        );
    }

    // Silence unused-import lint when only some tests compile helpers.
    #[allow(dead_code)]
    fn _keep(_: fn() -> ()) {
        let _ = attach_test_bridge_runtime;
    }
}
