use crate::api;
use crate::auth::{Login, Password};
use crate::api::response::Response;

use std::error::Error;
use std::io::Error as ioErr;
use std::io::ErrorKind as ioErrKind;
use serde_json::Value;
use sqlx::PgPool;

pub enum Operation {
    Create,
    Read,
    Update,
    Delete,
    Verify,
}

pub enum Target {
    Conversations,
    Messages,
    Users,
}

// Canonical form of a request
pub struct Request {
    pub operation: Operation,
    pub target: Target,
    users: Option<Vec<api::User>>,
    messages: Option<Vec<api::Message>>,
    conversations: Option<Vec<api::Conversation>>,
}

impl Request {
    fn split_function(function: &str) -> (String, String) {
        let split_func: Vec<&str> = function
            .split_ascii_whitespace()
            .collect();

        (split_func[0].to_owned(), split_func[1].to_owned())
    }

    pub fn from_json(data: &str) -> Result<Self, Box<dyn Error>> {
        let data: Value = serde_json::from_str(data)?;

        let (operation, target) = Self::split_function(data["function"].as_str()
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Invalid request function"))?);

        let request = Self{
            operation: match operation.as_ref() {
                "VERIFY" => Operation::Verify,
                "CREATE" => Operation::Create,
                "READ" => Operation::Read,
                "UPDATE" => Operation::Update,
                "DELETE" => Operation::Delete,
                _ => return Err(Box::new(ioErr::new(ioErrKind::InvalidInput, "Unknown request"))),
            },
            target: match target.as_ref() {
                "CONVERSATIONS" => Target::Conversations,
                "MESSAGES" => Target::Messages,
                "USERS" => Target::Users,
                _ => return Err(Box::new(ioErr::new(ioErrKind::InvalidInput, "Unknown target"))),
            },
            users: match data["users"].as_array() {
                Some(d) => {
                    let mut users = Vec::new();
                    for item in d.iter() {
                        let user = api::User::from_json(item)?;
                        users.push(user);
                    };
                    Some(users)
                },
                None => None,
            },
            messages: match data["messages"].as_array() {
                Some(d) => {
                    let mut messages = Vec::new();
                    for item in d.iter() {
                        let message = api::Message::from_json(item)?;
                        messages.push(message);
                    };
                    Some(messages)
                },
                None => None,
            },
            conversations: match data["conversations"].as_array() {
                Some(d) => {
                    let mut conversations = Vec::new();
                    for item in d.iter() {
                        let conversation = api::Conversation::from_json(item)?;
                        conversations.push(conversation);
                    };
                    Some(conversations)
                },
                None => None,
            },
        };

        Ok(request)
    }

    pub async fn verify_users(self, login: &mut Login, db_pool: &PgPool) -> Result<Response, Box<dyn Error>> {
        // Read remote data
        let users = self.users
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'users' list"))?;
        let user = users[0].clone();

        let email = user.email
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'email' field for 'user'"))?;
        let remote_pass = user.password
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'password' field for 'user'"))?;

        // Read local data
        let stream = sqlx::query_file!("src/sql/verify-user.sql", email)
            .fetch_one(db_pool)
            .await?;

        let local_pass = Password{
            hash: stream.pass,
            salt: stream.salt
        };

        // Validate password
        match local_pass.is_valid(&remote_pass)? {
            true => login.authenticate(email),
            false => return Err(Box::new(ioErr::new(ioErrKind::PermissionDenied, "Invalid password"))),
        };

        Ok(Response{
            status: 1,
            conversations: None,
            messages: None,
            users: None,
        })
    }

    pub async fn create_users(self, db_pool: &PgPool) -> Result<Response, Box<dyn Error>> {
        let users = self.users
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'users' list"))?;

        for user in users {
            let email = user.email
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'email' field for 'user'"))?;
            let password = user.password
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'password' field for 'user'"))?;
            let public_key = user.public_key
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'public_key' field for 'user'"))?;

            // Salt and hash password
            let password = Password::hash(&password, Option::None)?;

            // Store user data
            sqlx::query_file!("src/sql/create-user.sql",
                    email,
                    public_key,
                    password.hash,
                    password.salt)
                .execute(db_pool)
                .await?;
        };

        Ok(Response{
            status: 1,
            conversations: None,
            messages: None,
            users: None,
        })
    }

    pub async fn create_conversations(self, login: &Login, db_pool: &PgPool) -> Result<Response, Box<dyn Error>> {
        if login.is_authenticated == false {
            return Err(Box::new(ioErr::new(ioErrKind::PermissionDenied, "Not authenticated")));
        }

        let users = self.users
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'users' list"))?;
        let conversations = self.conversations
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'conversations' list"))?;

        let conversation = conversations[0].clone();

        let name = conversation.name
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'name' field for 'conversation'"))?;
        let name = std::str::from_utf8(&name)?;

        // Create conversation
        sqlx::query_file!("src/sql/create-conversation-1.sql", name)
            .execute(db_pool)
            .await?;

        // Add creator user
        sqlx::query_file!("src/sql/create-conversation-2.sql", login.email, name)
            .execute(db_pool)
            .await?;

        // Add remaining users
        for user in users.clone() {
            let email = user.email
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'email' field for 'user'"))?;

            sqlx::query_file!("src/sql/create-conversation-2.sql", email, name)
                .execute(db_pool)
                .await?;
        };

        Ok(Response{
            status: 1,
            conversations: None,
            messages: None,
            users: None,
        })
    }

    pub async fn create_messages(self, login: &Login, db_pool: &PgPool) -> Result<Response, Box<dyn Error>> {
        if login.is_authenticated == false {
            return Err(Box::new(ioErr::new(ioErrKind::PermissionDenied, "Not authenticated")));
        }

        let messages = self.messages
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'messages' list"))?;
        let conversations = self.conversations
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'conversations' list"))?;
        let conversation = conversations[0].clone();

        for message in messages {
            let data = message.data.clone()
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'data' field for 'message'"))?;
            let media_type = message.media_type.clone()
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'media_type' field for 'message'"))?;
            let timestamp = message.timestamp.clone()
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'timestamp' field for 'message'"))?;
            let signature = message.signature.clone()
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'signature' field for 'message'"))?;
            let conversation_id = conversation.id
                .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'id' field for 'message'"))?;

            // Store user data
            sqlx::query_file!("src/sql/create-message.sql",
                    login.email,
                    data,
                    media_type,
                    timestamp,
                    signature)
                .execute(db_pool)
                .await?;
        };
        
        Ok(Response{
            status: 1,
            conversations: None,
            messages: None,
            users: None,
        })
    }

    pub async fn read_conversations(self, login: &Login, db_pool: &PgPool) -> Result<Response, Box<dyn Error>> {
        if login.is_authenticated == false {
            return Err(Box::new(ioErr::new(ioErrKind::PermissionDenied, "Not authenticated")));
        }

        let stream = sqlx::query_file!("src/sql/read-conversation.sql", login.email)
            .fetch_one(db_pool)
            .await?;

        Ok(Response{
            status: 1,
            conversations: None,
            messages: None,
            users: None,
        })
    }

    pub async fn read_messages(self, login: &Login, db_pool: &PgPool) -> Result<Response, Box<dyn Error>> {
        if login.is_authenticated == false {
            return Err(Box::new(ioErr::new(ioErrKind::PermissionDenied, "Not authenticated")));
        }

        let conversations = self.conversations
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'conversations' list"))?;
        let conversation = conversations[0].clone();

        let id = conversation.id
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'id' field for 'conversation'"))?;

        let stream = sqlx::query_file!("src/sql/read-message.sql", login.email, id)
            .fetch_one(db_pool)
            .await?;

        Ok(Response{
            status: 1,
            conversations: None,
            messages: None,
            users: None,
        })
    }

    pub async fn read_users(self, login: &Login, db_pool: &PgPool) -> Result<Response, Box<dyn Error>> {
        if login.is_authenticated == false {
            return Err(Box::new(ioErr::new(ioErrKind::PermissionDenied, "Not authenticated")));
        }

        let conversations = self.conversations
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'conversations' list"))?;
        let conversation = conversations[0].clone();

        let id = conversation.id
            .ok_or_else(|| ioErr::new(ioErrKind::InvalidInput, "Missing 'id' field for 'conversation'"))?;

        let stream = sqlx::query_file!("src/sql/read-user.sql", login.email, id)
            .fetch_one(db_pool)
            .await?;

        Ok(Response{
            status: 1,
            conversations: None,
            messages: None,
            users: None,
        })
    }
}