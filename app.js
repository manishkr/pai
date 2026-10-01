(function () {
  const state = {
    auth: null,
    conversations: [],
    messages: [],
    documentTree: [],
    collapsedDocumentCategories: {},
    selectedConversationId: null,
    selectedDocumentIds: [],
    mentionQuery: null,
    mentionActiveIndex: 0,
    streaming: false,
    eventSource: null,
    streamClosedIntentionally: false
  };

  const nodes = {
    appLayout: document.querySelector("[data-app-layout]"),
    loginScreen: document.querySelector("[data-login-screen]"),
    conversationList: document.querySelector("[data-conversation-list]"),
    documentList: document.querySelector("[data-document-list]"),
    messages: document.querySelector("[data-messages]"),
    emptyState: document.querySelector("[data-empty-state]"),
    selectedDocs: document.querySelector("[data-selected-docs]"),
    mentionSuggestions: document.querySelector("[data-mention-suggestions]"),
    status: document.querySelector("[data-status]"),
    error: document.querySelector("[data-error]"),
    prompt: document.querySelector("[data-prompt]"),
    newChat: document.querySelector("[data-new-chat]"),
    form: document.querySelector("[data-composer]"),
    send: document.querySelector("[data-send]"),
    userName: document.querySelector("[data-user-name]"),
    userEmail: document.querySelector("[data-user-email]"),
    userAvatar: document.querySelector("[data-user-avatar]"),
    logout: document.querySelector("[data-logout]")
  };

  function setStatus(message) {
    nodes.status.textContent = message;
  }

  function setError(message) {
    if (!message) {
      nodes.error.textContent = "";
      nodes.error.classList.add("hidden");
      return;
    }

    nodes.error.textContent = message;
    nodes.error.classList.remove("hidden");
  }

  function escapeHtml(value) {
    return value
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll("\"", "&quot;")
      .replaceAll("'", "&#39;");
  }

  function formatEpoch(epoch) {
    if (!epoch || epoch <= 0) {
      return "Just now";
    }

    return new Date(epoch * 1000).toLocaleString();
  }

  function authInitials() {
    if (!state.auth) {
      return "SD";
    }

    const source = state.auth.name || state.auth.email;
    return source
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((part) => part[0].toUpperCase())
      .join("");
  }

  function selectedDocuments() {
    return flatDocuments().filter((doc) => state.selectedDocumentIds.includes(doc.id));
  }

  function flatDocuments() {
    return state.documentTree.flatMap((node) => node.children || []);
  }

  function renderShell() {
    const authenticated = Boolean(state.auth);
    nodes.loginScreen.classList.toggle("hidden", authenticated);
    nodes.appLayout.classList.toggle("hidden", !authenticated);

    if (!authenticated) {
      return;
    }

    nodes.userName.textContent = state.auth.name;
    nodes.userEmail.textContent = state.auth.email;
    nodes.userAvatar.textContent = authInitials();
  }

  function renderConversations() {
    nodes.conversationList.innerHTML = "";

    for (const conversation of state.conversations) {
      const button = document.createElement("button");
      button.className = "conversation-button";
      button.dataset.active = String(conversation.id === state.selectedConversationId);
      button.innerHTML = `
        <div class="conversation-title">${escapeHtml(conversation.title)}</div>
        <div class="conversation-time">${escapeHtml(formatEpoch(conversation.updated_at))}</div>
      `;
      button.addEventListener("click", () => {
        void selectConversation(conversation.id);
      });
      nodes.conversationList.appendChild(button);
    }
  }

  function renderDocumentRail() {
    nodes.documentList.innerHTML = "";

    for (const node of state.documentTree) {
      const isCollapsed = Boolean(state.collapsedDocumentCategories[node.id]);
      const section = document.createElement("section");
      section.className = "document-tree-section";
      section.innerHTML = `
        <button class="document-tree-parent" type="button" data-category-id="${escapeHtml(node.id)}" data-collapsed="${String(isCollapsed)}">
          <span class="document-tree-caret" aria-hidden="true">${isCollapsed ? "▸" : "▾"}</span>
          <span>${escapeHtml(node.title)}</span>
        </button>
        <div class="document-tree-children${isCollapsed ? " hidden" : ""}"></div>
      `;

      const parent = section.querySelector("[data-category-id]");
      const children = section.querySelector(".document-tree-children");
      parent.addEventListener("click", () => toggleDocumentCategory(node.id));

      for (const doc of node.children || []) {
        const button = document.createElement("button");
        button.className = "document-card";
        button.dataset.active = String(state.selectedDocumentIds.includes(doc.id));
        button.innerHTML = `
          <div class="document-card-title">${escapeHtml(doc.title)}</div>
          <div class="document-card-source">${escapeHtml(doc.source)}</div>
          <div class="document-card-summary">${escapeHtml(doc.summary)}</div>
        `;
        button.addEventListener("click", () => toggleDocumentSelection(doc.id));
        children.appendChild(button);
      }

      nodes.documentList.appendChild(section);
    }
  }

  function toggleDocumentCategory(categoryId) {
    state.collapsedDocumentCategories = {
      ...state.collapsedDocumentCategories,
      [categoryId]: !state.collapsedDocumentCategories[categoryId]
    };
    renderDocumentRail();
  }

  function renderSelectedDocuments() {
    const docs = selectedDocuments();
    if (docs.length === 0) {
      nodes.selectedDocs.innerHTML = "";
      nodes.selectedDocs.classList.add("hidden");
      return;
    }

    nodes.selectedDocs.classList.remove("hidden");
    nodes.selectedDocs.innerHTML = docs
      .map((doc) => `
        <button class="selected-doc-chip" type="button" data-remove-document="${escapeHtml(doc.id)}">
          @${escapeHtml(doc.title)}
          <span aria-hidden="true">&times;</span>
        </button>
      `)
      .join("");

    nodes.selectedDocs.querySelectorAll("[data-remove-document]").forEach((button) => {
      button.addEventListener("click", () => {
        toggleDocumentSelection(button.dataset.removeDocument);
      });
    });
  }

  function renderMessages(streamingText) {
    const items = [...state.messages];
    nodes.messages.innerHTML = "";

    if (streamingText) {
      items.push({
        role: "assistant",
        content: streamingText,
        id: "streaming"
      });
    }

    if (items.length === 0) {
      nodes.emptyState.classList.remove("hidden");
      return;
    }

    nodes.emptyState.classList.add("hidden");

    for (const message of items) {
      const role = message.role === "user" ? "user" : "assistant";
      const row = document.createElement("div");
      row.className = `message-row ${role}`;
      row.innerHTML = `
        ${role === "assistant" ? '<div class="message-avatar">AI</div>' : ""}
        <div class="message ${role === "user" ? "message-user" : "message-assistant"}"></div>
        ${role === "user" ? '<div class="message-avatar">U</div>' : ""}
      `;
      row.querySelector(".message").textContent = message.content;
      nodes.messages.appendChild(row);
    }

    nodes.messages.scrollTop = nodes.messages.scrollHeight;
  }

  function renderMentionSuggestions() {
    const matches = filteredMentionDocuments();
    if (!state.mentionQuery || matches.length === 0) {
      state.mentionActiveIndex = 0;
      nodes.mentionSuggestions.classList.add("hidden");
      nodes.mentionSuggestions.innerHTML = "";
      return;
    }

    if (state.mentionActiveIndex >= matches.length) {
      state.mentionActiveIndex = 0;
    }

    nodes.mentionSuggestions.classList.remove("hidden");
    nodes.mentionSuggestions.innerHTML = matches
      .map((doc, index) => `
        <button class="mention-option${index === state.mentionActiveIndex ? " mention-option-active" : ""}" type="button" data-mention-document="${escapeHtml(doc.id)}">
          <div class="mention-option-title">@${escapeHtml(doc.title)}</div>
          <div class="mention-option-copy">${escapeHtml(doc.source)}</div>
        </button>
      `)
      .join("");

    nodes.mentionSuggestions.querySelectorAll("[data-mention-document]").forEach((button) => {
      button.addEventListener("click", () => applyMention(button.dataset.mentionDocument));
    });
  }

  function filteredMentionDocuments() {
    if (!state.mentionQuery) {
      return [];
    }

    const term = state.mentionQuery.query.trim().toLowerCase();
    return flatDocuments()
      .filter((doc) => {
        if (!term) {
          return true;
        }

        return doc.title.toLowerCase().includes(term) || doc.source.toLowerCase().includes(term);
      })
      .slice(0, 5);
  }

  function updateMentionState() {
    const value = nodes.prompt.value;
    const caret = nodes.prompt.selectionStart || 0;
    const beforeCaret = value.slice(0, caret);
    const match = beforeCaret.match(/(^|\s)@([a-zA-Z0-9._ -]*)$/);

    if (!match) {
      state.mentionQuery = null;
      state.mentionActiveIndex = 0;
      renderMentionSuggestions();
      return;
    }

    const atIndex = beforeCaret.lastIndexOf("@");
    state.mentionQuery = {
      index: atIndex,
      query: match[2] || ""
    };
    state.mentionActiveIndex = 0;
    renderMentionSuggestions();
  }

  function applyMention(documentId) {
    const doc = flatDocuments().find((item) => item.id === documentId);
    if (!doc || !state.mentionQuery) {
      return;
    }

    const value = nodes.prompt.value;
    const selectionEnd = nodes.prompt.selectionStart || value.length;
    const before = value.slice(0, state.mentionQuery.index);
    const after = value.slice(selectionEnd);
    const mention = `@${doc.title} `;
    nodes.prompt.value = `${before}${mention}${after}`;

    const nextCaret = before.length + mention.length;
    nodes.prompt.focus();
    nodes.prompt.setSelectionRange(nextCaret, nextCaret);

    selectDocument(documentId);
    state.mentionQuery = null;
    state.mentionActiveIndex = 0;
    renderMentionSuggestions();
  }

  function selectDocument(documentId) {
    if (!documentId || state.selectedDocumentIds.includes(documentId)) {
      return;
    }

    state.selectedDocumentIds = [...state.selectedDocumentIds, documentId];
    renderDocumentRail();
    renderSelectedDocuments();
  }

  function toggleDocumentSelection(documentId) {
    if (!documentId) {
      return;
    }

    if (state.selectedDocumentIds.includes(documentId)) {
      state.selectedDocumentIds = state.selectedDocumentIds.filter((id) => id !== documentId);
    } else {
      state.selectedDocumentIds = [...state.selectedDocumentIds, documentId];
    }

    renderDocumentRail();
    renderSelectedDocuments();
  }

  async function readJson(response) {
    const contentType = response.headers.get("content-type") || "";
    const payload = contentType.includes("application/json") ? await response.json() : null;

    if (!response.ok) {
      const error = new Error(payload && payload.error ? payload.error : `Request failed with ${response.status}`);
      error.status = response.status;
      throw error;
    }

    return payload;
  }

  function handleUnauthorized() {
    closeStream();
    state.auth = null;
    state.conversations = [];
    state.messages = [];
    state.documentTree = [];
    state.selectedConversationId = null;
    state.selectedDocumentIds = [];
    state.mentionQuery = null;
    state.mentionActiveIndex = 0;
    state.streaming = false;
    nodes.prompt.value = "";
    setError("");
    renderShell();
    renderConversations();
    renderDocumentRail();
    renderSelectedDocuments();
    renderMentionSuggestions();
    renderMessages("");
  }

  async function authedJson(url, options) {
    try {
      return await readJson(await fetch(url, options));
    } catch (error) {
      if (error.status === 401) {
        handleUnauthorized();
      }
      throw error;
    }
  }

  async function fetchMe() {
    const payload = await authedJson("/api/me");
    state.auth = payload.user;
    renderShell();
  }

  async function fetchConversations() {
    const payload = await authedJson("/api/conversations");
    state.conversations = payload;
    renderConversations();
  }

  async function fetchDocuments() {
    const payload = await authedJson("/api/documents");
    state.documentTree = payload;
    state.selectedDocumentIds = state.selectedDocumentIds.filter((documentId) =>
      flatDocuments().some((document) => document.id === documentId)
    );
    renderDocumentRail();
    renderSelectedDocuments();
    renderMentionSuggestions();
  }

  async function fetchMessages(conversationId) {
    const payload = await authedJson(`/api/conversations/${conversationId}/messages`);
    state.messages = payload.messages;
    renderMessages("");
  }

  async function createConversation() {
    const payload = await authedJson("/api/conversations", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: null })
    });
    const conversation = payload.conversation;
    state.selectedConversationId = conversation.id;
    state.messages = [];
    state.conversations = [conversation, ...state.conversations.filter((item) => item.id !== conversation.id)];
    renderConversations();
    renderMessages("");
    return conversation.id;
  }

  async function selectConversation(conversationId) {
    state.selectedConversationId = conversationId;
    renderConversations();
    setError("");
    setStatus("Loading conversation...");

    try {
      await fetchMessages(conversationId);
      setStatus("Ready");
    } catch (error) {
      if (error.status !== 401) {
        setError(error.message);
        setStatus("Failed to load conversation");
      }
    }
  }

  function setStreaming(active) {
    state.streaming = active;
    nodes.send.disabled = active;
    nodes.newChat.disabled = active;
    setStatus(active ? "Assistant is streaming..." : "Ready");
  }

  function selectedDocumentQueryValue() {
    return state.selectedDocumentIds.join(",");
  }

  async function submitPrompt(event) {
    event.preventDefault();
    if (state.streaming || !state.auth) {
      return;
    }

    const prompt = nodes.prompt.value.trim();
    if (!prompt) {
      return;
    }

    setError("");

    try {
      let conversationId = state.selectedConversationId;
      if (!conversationId) {
        conversationId = await createConversation();
      }

      state.messages.push({
        id: `user-${Date.now()}`,
        conversation_id: conversationId,
        role: "user",
        content: prompt
      });
      renderMessages("");
      nodes.prompt.value = "";
      state.mentionQuery = null;
      renderMentionSuggestions();
      startStream(conversationId, prompt, selectedDocumentQueryValue());
    } catch (error) {
      if (error.status !== 401) {
        setError(error.message);
      }
    }
  }

  function closeStream() {
    if (state.eventSource) {
      state.streamClosedIntentionally = true;
      state.eventSource.close();
      state.eventSource = null;
    }
  }

  function startStream(conversationId, prompt, documentIds) {
    closeStream();
    setStreaming(true);
    state.streamClosedIntentionally = false;

    let streamingText = "";
    let sawCompletion = false;
    const params = new URLSearchParams({
      conversation_id: conversationId,
      message: prompt
    });
    if (documentIds) {
      params.set("document_ids", documentIds);
    }

    const eventSource = new EventSource(`/api/chat/stream?${params.toString()}`);
    state.eventSource = eventSource;

    eventSource.onmessage = async (event) => {
      const payload = JSON.parse(event.data);
      if (payload.kind === "token") {
        streamingText += payload.text;
        renderMessages(streamingText);
        return;
      }

      if (payload.kind === "completed") {
        sawCompletion = true;
        closeStream();
        setStreaming(false);
        await refreshConversationAfterStream(conversationId);
        return;
      }

      if (payload.kind === "error") {
        closeStream();
        setStreaming(false);
        if (payload.message.includes("unauthorized")) {
          handleUnauthorized();
          return;
        }
        setError(payload.message);
        renderMessages("");
      }
    };

    eventSource.onerror = async () => {
      if (state.streamClosedIntentionally) {
        state.streamClosedIntentionally = false;
        return;
      }

      closeStream();
      setStreaming(false);
      if (sawCompletion || streamingText) {
        try {
          await refreshConversationAfterStream(conversationId);
          return;
        } catch (_error) {
        }
      }

      setError("Streaming connection closed unexpectedly.");
    };
  }

  async function refreshConversationAfterStream(conversationId) {
    try {
      await Promise.all([fetchConversations(), fetchMessages(conversationId)]);
      state.selectedConversationId = conversationId;
      renderConversations();
    } catch (error) {
      if (error.status !== 401) {
        setError(error.message);
      }
    }
  }

  async function logout() {
    closeStream();

    try {
      const response = await fetch("/auth/logout", { method: "POST" });
      if (!response.ok && response.status !== 401) {
        throw new Error(`Logout failed with ${response.status}`);
      }
    } finally {
      handleUnauthorized();
    }
  }

  async function bootstrap() {
    try {
      await fetchMe();
    } catch (error) {
      if (error.status === 401) {
        handleUnauthorized();
        return;
      }

      setError(error.message);
      setStatus("Failed to initialize authentication");
      return;
    }

    try {
      await Promise.all([fetchConversations(), fetchDocuments()]);
      if (state.conversations.length > 0) {
        await selectConversation(state.conversations[0].id);
      } else {
        renderMessages("");
        setStatus("Ready");
      }
    } catch (error) {
      if (error.status !== 401) {
        setError(error.message);
        setStatus("Failed to load app data");
      }
    }
  }

  async function init() {
    nodes.newChat.addEventListener("click", async function () {
      if (state.streaming || !state.auth) {
        return;
      }

      setError("");
      try {
        await createConversation();
        setStatus("Ready");
      } catch (error) {
        if (error.status !== 401) {
          setError(error.message);
        }
      }
    });

    nodes.logout.addEventListener("click", function () {
      void logout();
    });

    nodes.form.addEventListener("submit", submitPrompt);
    nodes.prompt.addEventListener("input", updateMentionState);
    nodes.prompt.addEventListener("keydown", function (event) {
      if (event.key === "Enter" && !event.shiftKey) {
        if (state.mentionQuery) {
          const matches = filteredMentionDocuments();
          if (matches.length > 0) {
            event.preventDefault();
            applyMention(matches[state.mentionActiveIndex].id);
            return;
          }
        }

        event.preventDefault();
        nodes.form.requestSubmit();
        return;
      }

      if (event.key === "ArrowDown" && state.mentionQuery) {
        const matches = filteredMentionDocuments();
        if (matches.length > 0) {
          event.preventDefault();
          state.mentionActiveIndex = (state.mentionActiveIndex + 1) % matches.length;
          renderMentionSuggestions();
        }
        return;
      }

      if (event.key === "ArrowUp" && state.mentionQuery) {
        const matches = filteredMentionDocuments();
        if (matches.length > 0) {
          event.preventDefault();
          state.mentionActiveIndex =
            (state.mentionActiveIndex - 1 + matches.length) % matches.length;
          renderMentionSuggestions();
        }
        return;
      }

      if (event.key === "Escape") {
        state.mentionQuery = null;
        state.mentionActiveIndex = 0;
        renderMentionSuggestions();
        return;
      }

      if (event.key === "Tab") {
        const matches = filteredMentionDocuments();
        if (state.mentionQuery && matches.length > 0) {
          event.preventDefault();
          applyMention(matches[state.mentionActiveIndex].id);
        }
      }
    });

    handleUnauthorized();
    setStatus("Checking sign-in...");
    await bootstrap();
  }

  void init();
})();
