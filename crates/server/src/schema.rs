diesel::table! {
    auth_sessions (id) {
        id -> Uuid,
        user_id -> Uuid,
        expires_at -> BigInt,
        created_at -> BigInt,
    }
}

diesel::table! {
    conversations (id) {
        id -> Uuid,
        user_id -> Uuid,
        title -> Text,
        created_at -> BigInt,
        updated_at -> BigInt,
    }
}

diesel::table! {
    messages (id) {
        id -> Uuid,
        conversation_id -> Uuid,
        role -> Text,
        content -> Text,
        created_at -> BigInt,
    }
}

diesel::table! {
    users (id) {
        id -> Uuid,
        google_sub -> Text,
        email -> Text,
        name -> Text,
        avatar_url -> Nullable<Text>,
        hosted_domain -> Text,
        created_at -> BigInt,
        last_login_at -> BigInt,
    }
}

diesel::joinable!(auth_sessions -> users (user_id));
diesel::joinable!(messages -> conversations (conversation_id));
diesel::joinable!(conversations -> users (user_id));
diesel::allow_tables_to_appear_in_same_query!(auth_sessions, conversations, messages, users);
