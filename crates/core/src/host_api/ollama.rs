use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Queries Ollama running locally on the remote host (http://127.0.0.1:11434/api/generate)
    /// through the active SSH session and returns the generated command suggestion.
    pub async fn query_host_ollama(
        &self,
        model: String,
        system_prompt: String,
        user_prompt: String,
    ) -> Result<crate::ai::AiCommandSuggestion, SshCoreError> {
        self.query_ollama_on_port(11434, model, system_prompt, user_prompt)
            .await
    }

    /// One turn of a free-form chat with the host's Ollama: the whole history goes in, the
    /// assistant's reply comes back as plain text. Dropping the call stops the request on the host.
    pub async fn chat_host_ollama(
        &self,
        model: String,
        system_prompt: String,
        messages: Vec<crate::ai::ChatMessage>,
    ) -> Result<String, SshCoreError> {
        self.chat_ollama_on_port(11434, model, system_prompt, messages)
            .await
    }
}
