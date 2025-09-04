pub const DEFAULT_CONFIG_YAML: &str = r#"api_key: 
api_base: http://127.0.0.1:1234/v1
model: jade_qwen3_4b_mlx
context_max_tokens: 31768
assistant_minimum_context_tokens: 8192
should_stream: true
stop_words:
- |2-

  <|im_start|>
- <|im_end|>
session_db_url: "/Users/tg/Library/Application Support/com.awful-sec.aj/aj.db"
session_name: default
"#;

pub const DEFAULT_RUSTDOC_FN_YAML: &str = r#"system_prompt: You are Awful Jade, created by Awful Security.
messages: []
pre_user_message_content: |
  Here is a perfectly commented Rustdoc snippet for future reference. Please format your response exactly like it.
  /// Handle the `ask` subcommand.
  ///
  /// Loads the selected (or default) chat template and (optionally) a question.
  /// If a session is active in config, the function loads or initializes a per-session
  /// [`VectorStore`] and builds a [`Brain`] with a token budget of 25% of
  /// `context_max_tokens`. It then calls [`api::ask`], optionally passing both
  /// the vector store and brain so the API layer can inject retrieved memories.
  ///
  /// On success, the vector store is serialized back to disk for future queries.
  ///
  /// # Parameters
  /// - `jade_config`: Loaded [`config::AwfulJadeConfig`].
  /// - `question`: Optional question text. If `None`, defaults to
  ///   `"What is the meaning of life?"`.
  /// - `template_name`: Optional template name. If `None`, defaults to `"simple_question"`.
  ///
  /// # Errors
  /// - Returns I/O errors when loading/saving files,
  /// - YAML/JSON errors for (de)serialization,
  /// - and API/template loading errors bubbled up from the `awful_aj` crate.
  ///
  /// # Examples
  /// ```no_run
  /// # async fn example(cfg: awful_aj::config::AwfulJadeConfig)
  /// # -> Result<(), Box<dyn std::error::Error>> {
  /// // handle_ask_command(cfg, Some("Hi!".into()), Some("default".into())).await?;
  /// # Ok(()) }
  /// ```

  # Rules for properly formatted Rustdocs
  1. Start every line with ///
  2. Start with a description
  3. Then print the Parameters, Returns, Errors, Notes, and Examples in that order.
  4. Do not insert breaks between comment lines.
post_user_message_content: "Please write comprehensive Rustdocs for this function. Return only the Rustdoc comment block. /nothink"
should_stream: false
"#;

pub const DEFAULT_RUSTDOC_STRUCT_YAML: &str = r#"system_prompt: You are Awful Jade, created by Awful Security.
messages: []
pre_user_message_content: |
  Here is a perfectly commented Rustdoc snippet for future reference. Please format your response exactly like it.
  /// A single conversational memory item (role + content).
  ///
  /// This is the fundamental unit the brain stores. It’s deliberately small and serializable,
  /// so you can persist/restore or shuttle memories between components.
  ///
  /// # Examples
  /// ```rust
  /// use awful_aj::brain::Memory;
  /// use async_openai::types::Role;
  ///
  /// let m = Memory::new(Role::User, "Hello, world!".to_string());
  /// assert_eq!(m.role, Role::User);
  /// assert_eq!(m.content, "Hello, world!");
  /// ```

  # Rules for properly formatted Rustdocs
  1. Start every line with ///
  2. Start with a description
  3. Do not insert breaks between comment lines.
post_user_message_content: "Please write comprehensive Rustdocs for this struct. Return only the Rustdoc comment block. /nothink"
should_stream: false
response_format:
  name: rustdoc_struct_with_fields
  strict: true
  description: Represents Rustdoc for a struct and its fields.
  schema:
    type: object
    additionalProperties: false
    required:
      - struct_doc
      - fields
    properties:
      struct_doc:
        type: string
        description: Rustdoc for the struct (short 1–2 sentence summary). Every line must start with '///'.
        minLength: 1
        pattern: "^(///.*\\n?)+$"
      fields:
        type: array
        description: Array of per-field Rustdoc comments.
        items:
          type: object
          additionalProperties: false
          required:
            - name
            - doc
          properties:
            name:
              type: string
              description: Exact field name as it appears in the struct.
              minLength: 1
            doc:
              type: string
              description: Rustdoc for the field. Keep it short; each line must start with '///'.
              minLength: 1
              pattern: "^(///.*\\n?)+$"
"#;