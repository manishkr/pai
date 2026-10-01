DROP TABLE messages;
DROP TABLE conversations;
DROP TABLE auth_sessions;
DROP TABLE users;

CREATE TABLE conversations (
    id UUID PRIMARY KEY,
    session_id TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX conversations_session_id_updated_at_idx
    ON conversations (session_id, updated_at DESC);

CREATE TABLE messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX messages_conversation_created_at_idx
    ON messages (conversation_id, created_at ASC);
