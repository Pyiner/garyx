use std::collections::HashSet;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use garyx_models::provider::{AgentRunRequest, StreamEvent};
use garyx_router::{AgentDispatcher, MessageRouter, ThreadStore, bindings_from_value};
use tokio::sync::Mutex;
use tracing::warn;

use crate::dispatcher::{
    ChannelDispatcher, StreamDispatchRole, StreamingDispatchTarget, build_stream_dispatch_callback,
};

type StreamCallback = Arc<dyn Fn(StreamEvent) + Send + Sync>;

fn binding_delivery_thread_id(binding_key: &str, chat_id: &str) -> Option<String> {
    let binding_key = binding_key.trim();
    let chat_id = chat_id.trim();
    if binding_key.is_empty() || binding_key == chat_id {
        None
    } else {
        Some(binding_key.to_owned())
    }
}

async fn snapshot_bound_targets(
    thread_store: Arc<dyn ThreadStore>,
    thread_id: &str,
    run_id: &str,
) -> Vec<StreamingDispatchTarget> {
    let Some(thread_data) = thread_store.get(thread_id).await else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for binding in bindings_from_value(&thread_data) {
        let channel = binding.channel.trim();
        let account_id = binding.account_id.trim();
        if channel.is_empty() || account_id.is_empty() || channel.eq_ignore_ascii_case("api") {
            continue;
        }

        let chat_id = binding.chat_id.trim().to_owned();
        let binding_key = binding.binding_key.trim().to_owned();
        let resolved_chat_id = if chat_id.is_empty() {
            binding_key.clone()
        } else {
            chat_id
        };
        if resolved_chat_id.is_empty() {
            continue;
        }

        let endpoint_identity = binding.endpoint_key();
        if !seen.insert(endpoint_identity.clone()) {
            continue;
        }

        targets.push(StreamingDispatchTarget {
            target_thread_id: thread_id.to_owned(),
            endpoint_identity,
            run_id: run_id.to_owned(),
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            chat_id: resolved_chat_id,
            delivery_target_type: binding.resolved_delivery_target_type(),
            delivery_target_id: binding.resolved_delivery_target_id(),
            thread_id: binding_delivery_thread_id(&binding.binding_key, &binding.chat_id),
        });
    }
    targets
}

#[derive(Default)]
struct DeferredState {
    callbacks: Option<Vec<StreamCallback>>,
    buffered: Vec<StreamEvent>,
}

struct DeferredBoundStreamFanoutInner {
    router: Arc<Mutex<MessageRouter>>,
    dispatcher: Arc<dyn ChannelDispatcher>,
    run_id: String,
    origin_endpoint_identity: String,
    state: StdMutex<DeferredState>,
}

#[derive(Clone)]
pub struct DeferredBoundStreamFanout {
    inner: Arc<DeferredBoundStreamFanoutInner>,
}

impl DeferredBoundStreamFanout {
    pub fn new(
        router: Arc<Mutex<MessageRouter>>,
        dispatcher: Arc<dyn ChannelDispatcher>,
        run_id: impl Into<String>,
        origin_endpoint_identity: impl Into<String>,
    ) -> Self {
        Self {
            inner: Arc::new(DeferredBoundStreamFanoutInner {
                router,
                dispatcher,
                run_id: run_id.into(),
                origin_endpoint_identity: origin_endpoint_identity.into(),
                state: StdMutex::new(DeferredState::default()),
            }),
        }
    }

    pub fn consumer(
        &self,
        origin_callback: Arc<dyn Fn(StreamEvent) + Send + Sync>,
    ) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
        let inner = self.inner.clone();
        Arc::new(move |event: StreamEvent| {
            origin_callback(event.clone());
            inner.dispatch_or_buffer(event);
        })
    }

    pub async fn attach_thread(&self, thread_id: &str) {
        let thread_store = {
            let router = self.inner.router.lock().await;
            router.thread_store()
        };
        self.attach_thread_from_store(thread_store, thread_id).await;
    }

    pub async fn attach_thread_from_store(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        thread_id: &str,
    ) {
        let already_attached = match self.inner.state.lock() {
            Ok(state) => state.callbacks.is_some(),
            Err(_) => {
                warn!("deferred bound stream fanout state lock poisoned");
                return;
            }
        };
        if already_attached {
            return;
        }

        let mut targets = snapshot_bound_targets(thread_store, thread_id, &self.inner.run_id).await;
        let origin = self.inner.origin_endpoint_identity.trim();
        targets.retain(|target| target.endpoint_identity.trim() != origin);

        let callbacks: Vec<_> = targets
            .into_iter()
            .filter_map(|target| {
                let callback = build_stream_dispatch_callback(
                    self.inner.dispatcher.clone(),
                    target.clone(),
                    self.inner.router.clone(),
                    StreamDispatchRole::BoundTarget,
                );
                if callback.is_none() {
                    warn!(
                        channel = %target.channel,
                        account_id = %target.account_id,
                        endpoint_identity = %target.endpoint_identity,
                        "no stream dispatch callback available for bound target"
                    );
                }
                callback
            })
            .collect();

        loop {
            let buffered = {
                let mut state = match self.inner.state.lock() {
                    Ok(state) => state,
                    Err(_) => {
                        warn!("deferred bound stream fanout state lock poisoned");
                        return;
                    }
                };
                if state.callbacks.is_some() {
                    return;
                }
                if state.buffered.is_empty() {
                    state.callbacks = Some(callbacks.clone());
                    return;
                }
                std::mem::take(&mut state.buffered)
            };

            for event in buffered {
                for callback in &callbacks {
                    callback(event.clone());
                }
            }
        }
    }
}

pub struct DeferredFanoutAgentDispatcher<'a> {
    inner: &'a dyn AgentDispatcher,
    fanout: DeferredBoundStreamFanout,
    thread_store: Arc<dyn ThreadStore>,
}

impl<'a> DeferredFanoutAgentDispatcher<'a> {
    pub fn new(
        inner: &'a dyn AgentDispatcher,
        fanout: DeferredBoundStreamFanout,
        thread_store: Arc<dyn ThreadStore>,
    ) -> Self {
        Self {
            inner,
            fanout,
            thread_store,
        }
    }
}

#[async_trait]
impl AgentDispatcher for DeferredFanoutAgentDispatcher<'_> {
    async fn dispatch(
        &self,
        request: AgentRunRequest,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<garyx_models::provider::AgentDispatchOutcome, String> {
        self.fanout
            .attach_thread_from_store(self.thread_store.clone(), &request.thread_id)
            .await;
        self.inner.dispatch(request, response_callback).await
    }
}

impl DeferredBoundStreamFanoutInner {
    fn dispatch_or_buffer(&self, event: StreamEvent) {
        let callbacks = {
            let mut state = match self.state.lock() {
                Ok(state) => state,
                Err(_) => {
                    warn!("deferred bound stream fanout state lock poisoned");
                    return;
                }
            };
            match state.callbacks.as_ref() {
                Some(callbacks) => callbacks.clone(),
                None => {
                    state.buffered.push(event);
                    return;
                }
            }
        };

        for callback in callbacks {
            callback(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;

    use garyx_models::config::GaryxConfig;
    use garyx_router::{InMemoryThreadStore, MessageRouter, ThreadStore};
    use serde_json::json;

    #[derive(Default)]
    struct RecordingDispatcher {
        events: Arc<StdMutex<Vec<(String, StreamEvent)>>>,
    }

    #[async_trait]
    impl ChannelDispatcher for RecordingDispatcher {
        async fn send_message(
            &self,
            _request: crate::dispatcher::OutboundMessage,
        ) -> Result<crate::dispatcher::SendMessageResult, crate::channel_trait::ChannelError>
        {
            unreachable!("test dispatcher should use native stream callback")
        }

        fn available_channels(&self) -> Vec<crate::dispatcher::ChannelInfo> {
            Vec::new()
        }

        fn build_stream_event_callback(
            &self,
            _target: StreamingDispatchTarget,
            _router: Arc<Mutex<MessageRouter>>,
        ) -> Option<crate::dispatcher::StreamDispatchCallback> {
            let events = self.events.clone();
            Some(Arc::new(move |envelope| {
                events
                    .lock()
                    .expect("events lock")
                    .push((envelope.endpoint_identity.clone(), envelope.event));
            }))
        }
    }

    struct MutatingAgentDispatcher {
        store: Arc<dyn ThreadStore>,
    }

    #[async_trait]
    impl AgentDispatcher for MutatingAgentDispatcher {
        async fn dispatch(
            &self,
            request: AgentRunRequest,
            response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
        ) -> Result<garyx_models::provider::AgentDispatchOutcome, String> {
            self.store
                .set(
                    &request.thread_id,
                    json!({
                        "channel_bindings": [
                            {
                                "channel": "telegram",
                                "account_id": "bot1",
                                "binding_key": "origin",
                                "chat_id": "chat-a",
                                "delivery_target_type": "chat_id",
                                "delivery_target_id": "chat-a"
                            },
                            {
                                "channel": "discord",
                                "account_id": "bot2",
                                "binding_key": "first",
                                "chat_id": "chat-b",
                                "delivery_target_type": "chat_id",
                                "delivery_target_id": "chat-b"
                            },
                            {
                                "channel": "feishu",
                                "account_id": "bot3",
                                "binding_key": "late",
                                "chat_id": "chat-c",
                                "delivery_target_type": "chat_id",
                                "delivery_target_id": "chat-c"
                            }
                        ]
                    }),
                )
                .await;
            if let Some(callback) = response_callback {
                callback(StreamEvent::Delta {
                    text: "after attach".to_owned(),
                });
            }
            Ok(garyx_models::provider::AgentDispatchOutcome::Started)
        }
    }

    #[tokio::test]
    async fn deferred_fanout_buffers_until_thread_attaches_and_excludes_origin_identity() {
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        store
            .set(
                "thread::bound",
                json!({
                    "channel_bindings": [
                        {
                            "channel": "telegram",
                            "account_id": "bot1",
                            "binding_key": "origin",
                            "chat_id": "chat-a",
                            "delivery_target_type": "chat_id",
                            "delivery_target_id": "chat-a"
                        },
                        {
                            "channel": "discord",
                            "account_id": "bot2",
                            "binding_key": "other",
                            "chat_id": "chat-b",
                            "delivery_target_type": "chat_id",
                            "delivery_target_id": "chat-b"
                        }
                    ]
                }),
            )
            .await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store,
            GaryxConfig::default(),
        )));
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let origin_events = Arc::new(StdMutex::new(Vec::<StreamEvent>::new()));
        let origin_callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = {
            let origin_events = origin_events.clone();
            Arc::new(move |event| {
                origin_events.lock().expect("origin lock").push(event);
            })
        };
        let fanout = DeferredBoundStreamFanout::new(
            router,
            dispatcher.clone(),
            "run-1",
            "telegram::bot1::origin",
        );
        let consumer = fanout.consumer(origin_callback);

        consumer(StreamEvent::Delta { text: "hi".into() });
        assert_eq!(origin_events.lock().expect("origin lock").len(), 1);
        assert!(dispatcher.events.lock().expect("events lock").is_empty());

        fanout.attach_thread("thread::bound").await;

        let delivered = dispatcher.events.lock().expect("events lock").clone();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].0, "discord::bot2::other");
        assert!(matches!(delivered[0].1, StreamEvent::Delta { ref text } if text == "hi"));

        consumer(StreamEvent::Done);
        let delivered = dispatcher.events.lock().expect("events lock").clone();
        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[1].0, "discord::bot2::other");
        assert!(matches!(delivered[1].1, StreamEvent::Done));
    }

    #[tokio::test]
    async fn agent_dispatcher_attaches_before_provider_dispatch_snapshots_bindings() {
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        store
            .set(
                "thread::bound",
                json!({
                    "channel_bindings": [
                        {
                            "channel": "telegram",
                            "account_id": "bot1",
                            "binding_key": "origin",
                            "chat_id": "chat-a",
                            "delivery_target_type": "chat_id",
                            "delivery_target_id": "chat-a"
                        },
                        {
                            "channel": "discord",
                            "account_id": "bot2",
                            "binding_key": "first",
                            "chat_id": "chat-b",
                            "delivery_target_type": "chat_id",
                            "delivery_target_id": "chat-b"
                        }
                    ]
                }),
            )
            .await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let origin_callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(|_| {});
        let fanout = DeferredBoundStreamFanout::new(
            router,
            dispatcher.clone(),
            "run-1",
            "telegram::bot1::origin",
        );
        let consumer = fanout.consumer(origin_callback);
        let inner = MutatingAgentDispatcher {
            store: store.clone(),
        };
        let attaching_dispatcher =
            DeferredFanoutAgentDispatcher::new(&inner, fanout.clone(), store);

        attaching_dispatcher
            .dispatch(
                AgentRunRequest::new(
                    "thread::bound",
                    "hello",
                    "run-1",
                    "telegram",
                    "bot1",
                    HashMap::new(),
                ),
                Some(consumer),
            )
            .await
            .unwrap();

        let delivered = dispatcher.events.lock().expect("events lock").clone();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].0, "discord::bot2::first");
        assert!(
            matches!(delivered[0].1, StreamEvent::Delta { ref text } if text == "after attach")
        );
    }
}
