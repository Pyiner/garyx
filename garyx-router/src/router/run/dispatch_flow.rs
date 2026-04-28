use super::super::inbound::InboundCommandClassifier;
use super::super::*;
impl MessageRouter {
    // ------------------------------------------------------------------
    // High-level route-and-dispatch (FR-1)
    // ------------------------------------------------------------------

    /// Resolve thread, enrich metadata, and dispatch to the provider
    /// bridge in a single call.
    ///
    /// This is the primary API for channel handlers (AC-1). It:
    ///
    /// 1. Resolves the thread id (via reply routing or inbound routing)
    /// 2. Checks for auto-recovery redirect
    /// 3. Enriches metadata with channel context
    /// 4. Updates last-delivery context for scheduled sends
    /// 5. Wraps the response callback to record outbound messages
    /// 6. Dispatches to the bridge
    ///
    /// Returns the resolved thread id and enriched metadata.
    pub async fn route_and_dispatch(
        &mut self,
        mut request: InboundRequest,
        dispatcher: &dyn AgentDispatcher,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<InboundResult, String> {
        if let Some(sink) = &self.inbound_sink
            && let Some(result) = sink.try_handle(&request).await
        {
            return result;
        }

        if let Some(command_text) = InboundCommandClassifier::command_text(&request) {
            if let Some(command) = InboundCommandClassifier::parse(command_text, &request.channel) {
                return self.handle_local_command(&request, command).await;
            }
        }

        self.apply_custom_command_transform(&mut request, None);

        let plan = self.build_dispatch_plan(request.into()).await;
        self.execute_dispatch_plan(
            plan,
            dispatcher,
            response_callback,
            "dispatching inbound run",
        )
        .await
    }

    pub async fn dispatch_message_to_thread(
        &mut self,
        thread_id: &str,
        request: ThreadMessageRequest,
        dispatcher: &dyn AgentDispatcher,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<InboundResult, String> {
        let mut context = self
            .build_thread_dispatch_context(thread_id, request)
            .await?;
        self.apply_custom_thread_message_transform(&mut context, Some(thread_id));

        let plan = self
            .build_dispatch_plan_for_thread(context, thread_id.to_owned(), false)
            .await;
        self.execute_dispatch_plan(
            plan,
            dispatcher,
            response_callback,
            "dispatching direct thread run",
        )
        .await
    }
}
