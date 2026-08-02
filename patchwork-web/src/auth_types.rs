use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    pub uuid: String,
    pub nickname: String,
    pub email: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedProjectDto {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub downloads: i64,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub repository_url: Option<String>,
    #[serde(default)]
    pub repository_path: Option<String>,
    #[serde(default)]
    pub can_rescan: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GithubAccountDto {
    pub github_user_id: i64,
    pub github_login: String,
    pub github_avatar_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubConnectRequest {
    pub completion_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubConnectResponse {
    pub authorization_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDto {
    pub account: AccountDto,
    pub github: Option<GithubAccountDto>,
    pub mods: Vec<PublishedProjectDto>,
    pub modpacks: Vec<PublishedProjectDto>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    pub email: String,
    pub nickname: String,
    pub password_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationChallengeDto {
    pub verification_id: String,
    pub email: String,
    pub expires_in: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRegistrationRequest {
    pub verification_id: String,
    pub code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub identifier: String,
    pub password_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNicknameRequest {
    pub nickname: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthTokenRequest {
    pub grant_type: String,
    pub client_id: String,
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub profile: ProfileDto,
}
