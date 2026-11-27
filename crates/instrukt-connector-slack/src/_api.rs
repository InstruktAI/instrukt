//! The Slack SDK sessions and the admission request that needs response headers,
//! natively async on the shared Tokio runtime with every call judged by `Gate`.
use std::future::Future;
use std::sync::Arc;

use hyper_rustls::HttpsConnectorBuilder;
use instrukt_connector::{AnswerReceipt, DirectMessageReceipt};
use instrukt_outside::http::{ClientConfig, build_client};
use instrukt_outside::{Attempted, Settings, Tried};
use serde::Deserialize;
use serde_json::Value;
use slack_morphism::hyper_tokio::{SlackClientHyperConnector, SlackClientHyperHttpsConnector};
use slack_morphism::prelude::{
    SlackApiAppsConnectionOpenRequest, SlackApiAssistantThreadsSetStatusRequest,
    SlackApiChatPostMessageRequest, SlackApiUsersInfoRequest,
};
use slack_morphism::{
    ClientResult, SlackApiToken, SlackChannelId, SlackClient, SlackClientSession,
    SlackMessageContent, SlackMessageMetadata, SlackTs,
};

use crate::_outside::{admission_answer, admission_transport, sdk_answer, token_key};
use crate::{
    Me, STATUS_TIMEOUT, SlackApi, SlackEndpoint, SlackError, WEB_API_TIMEOUT, split_conversation,
    ts_time,
};

pub(crate) type Client = SlackClient<SlackClientHyperHttpsConnector>;
type Session<'a> = SlackClientSession<'a, SlackClientHyperHttpsConnector>;

#[derive(Deserialize)]
struct Identity {
    user_id: String,
    team_id: String,
    bot_id: Option<String>,
}

#[derive(Deserialize)]
struct Posted {
    channel: Option<String>,
    ts: Option<String>,
}

impl SlackApi {
    pub(crate) fn new(
        company: &str,
        connector: &str,
        endpoint: &SlackEndpoint,
        app_token: String,
        bot_token: String,
    ) -> Result<Self, SlackError> {
        instrukt_outside::http::ensure_crypto_provider();
        let tls_builder = HttpsConnectorBuilder::new()
            .with_native_roots()
            .map_err(|error| api_error("client setup", error))?;
        let tls = if endpoint.local {
            tls_builder
                .https_or_http()
                .enable_http1()
                .enable_http2()
                .build()
        } else {
            tls_builder
                .https_only()
                .enable_http1()
                .enable_http2()
                .build()
        };
        let sdk_connector =
            SlackClientHyperConnector::with_connector(tls).with_slack_api_url(&endpoint.api);
        let admission_client = build_client(&ClientConfig {
            request_timeout: WEB_API_TIMEOUT,
            follow_redirects: false,
            ..ClientConfig::default()
        })
        .map_err(|error| api_error("admission client setup", error))?;

        Ok(Self {
            company: company.to_owned(),
            connector: connector.to_owned(),
            endpoint: endpoint.clone(),
            app_token: SlackApiToken::new(app_token.into()),
            bot_token: SlackApiToken::new(bot_token.into()),
            client: Arc::new(SlackClient::new(sdk_connector)),
            admission_client,
        })
    }

    fn bot_session(&self) -> Session<'_> {
        self.client.open_session(&self.bot_token)
    }

    fn app_session(&self) -> Session<'_> {
        self.client.open_session(&self.app_token)
    }

    async fn bot_call<T: Send>(
        &self,
        method: &str,
        call: impl Future<Output = ClientResult<T>> + Send,
    ) -> Result<T, SlackError> {
        self.sdk_call(false, method, call).await
    }

    async fn app_call<T: Send>(
        &self,
        method: &str,
        call: impl Future<Output = ClientResult<T>> + Send,
    ) -> Result<T, SlackError> {
        self.sdk_call(true, method, call).await
    }

    async fn sdk_call<T: Send>(
        &self,
        app: bool,
        method: &str,
        call: impl Future<Output = ClientResult<T>> + Send,
    ) -> Result<T, SlackError> {
        self.attempt(app, method, async {
            let timeout = if method == "assistant.threads.setStatus" {
                STATUS_TIMEOUT
            } else {
                WEB_API_TIMEOUT
            };
            sdk_answer(
                method,
                tokio::time::timeout(timeout, call).await,
                self.endpoint.gate.now(),
            )
        })
        .await
    }

    async fn attempt<T: Send>(
        &self,
        app: bool,
        method: &str,
        call: impl Future<Output = Tried<T, SlackError>> + Send,
    ) -> Result<T, SlackError> {
        let gate = &self.endpoint.gate;
        let key = token_key(&self.company, &self.connector, app).item(method);
        match gate
            .attempt_async(&key, &Settings::default(), || call)
            .await
        {
            Attempted::Done(value) => Ok(value),
            Attempted::Defect(error) => Err(error),
            Attempted::Wait { wait, error } => Err(SlackError::Unavailable {
                method: method.into(),
                cause: error.map(|error| match error {
                    SlackError::Api { cause, .. } => cause,
                    other => other.to_string(),
                }),
                wait,
            }),
        }
    }

    /// Ask Slack for one single-use Socket Mode WebSocket URL under the app
    /// token, judged by the outside gate and verified against `admits_socket`.
    pub(crate) async fn connection_url(&self) -> Result<String, SlackError> {
        const METHOD: &str = "apps.connections.open";
        let opened = self
            .app_call(
                METHOD,
                self.app_session()
                    .apps_connections_open(&SlackApiAppsConnectionOpenRequest::new()),
            )
            .await?;
        let url = opened.url.0.to_string();
        if !self.endpoint.admits_socket(&url) {
            return Err(SlackError::Socket);
        }
        Ok(url)
    }

    pub(crate) async fn check_scopes(&self, required: &[String]) -> Result<(), SlackError> {
        const METHOD: &str = "auth.test";
        self.attempt(false, METHOD, async {
            let response = self
                .admission_client
                .post(format!("{}/{METHOD}", self.endpoint.api))
                .bearer_auth(&self.bot_token.token_value.0)
                .send()
                .await;
            let received_at = self.endpoint.gate.now();
            let response = match response {
                Ok(response) => response,
                Err(error) => return admission_transport(None, received_at, &error),
            };
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            match response.text().await {
                Ok(body) => admission_answer(status, &headers, received_at, &body, required),
                Err(error) => admission_transport(Some((status, &headers)), received_at, &error),
            }
        })
        .await
    }

    pub(crate) async fn identity(&self) -> Result<Me, SlackError> {
        const METHOD: &str = "auth.test";
        // Identity needs only the installation IDs, which also work when
        // Slack omits the workspace's display information.
        let session = self.bot_session();
        let identity: Identity = self
            .bot_call(
                METHOD,
                session
                    .http_session_api
                    .http_get(METHOD, &[] as &[(&str, Option<&str>); 0], None),
            )
            .await?;
        if identity.user_id.is_empty() || identity.team_id.is_empty() {
            return Err(api_error(METHOD, "missing installation IDs"));
        }
        Ok(Me {
            user: identity.user_id,
            team: identity.team_id,
            bot: identity.bot_id,
        })
    }

    pub(crate) async fn user(&self, user: &str) -> Result<Value, SlackError> {
        let result = self
            .bot_call(
                "users.info",
                self.bot_session()
                    .users_info(&SlackApiUsersInfoRequest::new(user.into())),
            )
            .await?;
        serde_json::to_value(result.user).map_err(|error| api_error("users.info", error))
    }

    /// Keep the full conversation response for unmodelled Slack fields,
    /// including DM partners and message subtypes published after this SDK.
    pub(crate) async fn read(
        &self,
        method: &str,
        args: &[(&str, &str)],
    ) -> Result<Value, SlackError> {
        let params: Vec<_> = args
            .iter()
            .map(|&(name, value)| (name, Some(value)))
            .collect();
        self.bot_call(
            method,
            self.bot_session()
                .http_session_api
                .http_get(method, &params, None),
        )
        .await
    }

    pub(crate) async fn status(
        &self,
        channel: &str,
        thread: &str,
        status: &str,
    ) -> Result<(), SlackError> {
        let request = SlackApiAssistantThreadsSetStatusRequest::new(
            channel.into(),
            status.into(),
            thread.into(),
        );
        self.bot_call(
            "assistant.threads.setStatus",
            self.bot_session().assistant_threads_set_status(&request),
        )
        .await
        .map(drop)
    }

    async fn post_content(
        &self,
        channel: SlackChannelId,
        thread: Option<SlackTs>,
        content: SlackMessageContent,
    ) -> Result<Posted, SlackError> {
        const METHOD: &str = "chat.postMessage";
        let request = SlackApiChatPostMessageRequest::new(channel, content).opt_thread_ts(thread);
        // A successful post need not include enough evidence for a receipt.
        self.bot_call(
            METHOD,
            self.bot_session()
                .http_session_api
                .http_post(METHOD, &request, None),
        )
        .await
    }

    /// Post Slack-rendered Markdown to a channel thread or direct conversation.
    pub async fn answer(
        &self,
        conversation: &str,
        markdown: &str,
    ) -> Result<AnswerReceipt, SlackError> {
        let (channel, thread) = split_conversation(conversation);
        let content = SlackMessageContent::new().with_markdown_text(markdown.into());
        let post = self
            .post_content(channel.into(), thread.map(Into::into), content)
            .await?;
        Ok(AnswerReceipt {
            posted_at: post.ts.as_deref().and_then(ts_time),
            message_id: post.ts,
        })
    }

    /// Post plain text to a channel thread or direct conversation.
    pub async fn post(&self, conversation: &str, text: &str) -> Result<(), SlackError> {
        let (channel, thread) = split_conversation(conversation);
        let content = SlackMessageContent::new().with_text(text.into());
        self.post_content(channel.into(), thread.map(Into::into), content)
            .await
            .map(drop)
    }

    /// Post directly to a Slack user ID and mark the DM private so it is
    /// excluded from context reads.
    pub async fn post_private_dm(
        &self,
        recipient: &str,
        text: &str,
    ) -> Result<DirectMessageReceipt, SlackError> {
        let metadata = SlackMessageMetadata::new("instrukt_private".into())
            .with_event_payload(Default::default());
        let content = SlackMessageContent::new()
            .with_text(text.into())
            .with_metadata(metadata);
        let post = self.post_content(recipient.into(), None, content).await?;
        Ok(DirectMessageReceipt {
            channel: post.channel,
            message_id: post.ts,
        })
    }
}

fn api_error(method: &str, cause: impl std::fmt::Display) -> SlackError {
    SlackError::Api {
        method: method.into(),
        cause: cause.to_string(),
    }
}
