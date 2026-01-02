//! Admin API 业务逻辑服务

use std::sync::Arc;

use crate::kiro::token_manager::MultiTokenManager;

use super::error::AdminServiceError;
use super::types::{BalanceResponse, CredentialStatusItem, CredentialsStatusResponse};

/// Admin 服务
///
/// 封装所有 Admin API 的业务逻辑
pub struct AdminService {
    token_manager: Arc<MultiTokenManager>,
}

impl AdminService {
    pub fn new(token_manager: Arc<MultiTokenManager>) -> Self {
        Self { token_manager }
    }

    /// 获取所有凭据状态
    pub fn get_all_credentials(&self) -> CredentialsStatusResponse {
        let snapshot = self.token_manager.snapshot();

        let credentials: Vec<CredentialStatusItem> = snapshot
            .entries
            .into_iter()
            .map(|entry| CredentialStatusItem {
                index: entry.index,
                priority: entry.priority,
                disabled: entry.disabled,
                failure_count: entry.failure_count,
                is_current: entry.index == snapshot.current_index,
                expires_at: entry.expires_at,
                auth_method: entry.auth_method,
                has_profile_arn: entry.has_profile_arn,
            })
            .collect();

        CredentialsStatusResponse {
            total: snapshot.total,
            available: snapshot.available,
            current_index: snapshot.current_index,
            credentials,
        }
    }

    /// 设置凭据禁用状态
    pub fn set_disabled(&self, index: usize, disabled: bool) -> Result<(), AdminServiceError> {
        // 先获取当前凭据索引，用于判断是否需要切换
        let snapshot = self.token_manager.snapshot();
        let current_index = snapshot.current_index;
        let total = snapshot.total;

        self.token_manager
            .set_disabled(index, disabled)
            .map_err(|e| self.classify_error(e, index, total))?;

        // 只有禁用的是当前凭据时才尝试切换到下一个
        if disabled && index == current_index {
            let _ = self.token_manager.switch_to_next();
        }
        Ok(())
    }

    /// 设置凭据优先级
    pub fn set_priority(&self, index: usize, priority: u32) -> Result<(), AdminServiceError> {
        let total = self.token_manager.snapshot().total;
        self.token_manager
            .set_priority(index, priority)
            .map_err(|e| self.classify_error(e, index, total))
    }

    /// 重置失败计数并重新启用
    pub fn reset_and_enable(&self, index: usize) -> Result<(), AdminServiceError> {
        let total = self.token_manager.snapshot().total;
        self.token_manager
            .reset_and_enable(index)
            .map_err(|e| self.classify_error(e, index, total))
    }

    /// 获取凭据余额
    pub async fn get_balance(&self, index: usize) -> Result<BalanceResponse, AdminServiceError> {
        let total = self.token_manager.snapshot().total;
        let usage = self
            .token_manager
            .get_usage_limits_for(index)
            .await
            .map_err(|e| self.classify_balance_error(e, index, total))?;

        let current_usage = usage.current_usage();
        let usage_limit = usage.usage_limit();
        let remaining = (usage_limit - current_usage).max(0.0);
        let usage_percentage = if usage_limit > 0.0 {
            (current_usage / usage_limit * 100.0).min(100.0)
        } else {
            0.0
        };

        Ok(BalanceResponse {
            index,
            subscription_title: usage.subscription_title().map(|s| s.to_string()),
            current_usage,
            usage_limit,
            remaining,
            usage_percentage,
            next_reset_at: usage.next_date_reset,
        })
    }

    /// 分类简单操作错误（set_disabled, set_priority, reset_and_enable）
    fn classify_error(
        &self,
        e: anyhow::Error,
        index: usize,
        total: usize,
    ) -> AdminServiceError {
        let msg = e.to_string();
        if msg.contains("索引超出范围") {
            AdminServiceError::NotFound { index, total }
        } else {
            AdminServiceError::InternalError(msg)
        }
    }

    /// 分类余额查询错误（可能涉及上游 API 调用）
    fn classify_balance_error(
        &self,
        e: anyhow::Error,
        index: usize,
        total: usize,
    ) -> AdminServiceError {
        let msg = e.to_string();

        // 1. 索引越界
        if msg.contains("索引超出范围") {
            return AdminServiceError::NotFound { index, total };
        }

        // 2. 上游服务错误特征：HTTP 响应错误或网络错误
        let is_upstream_error =
            // HTTP 响应错误（来自 refresh_*_token 的错误消息）
            msg.contains("凭证已过期或无效") ||
            msg.contains("权限不足") ||
            msg.contains("已被限流") ||
            msg.contains("服务器错误") ||
            msg.contains("Token 刷新失败") ||
            msg.contains("暂时不可用") ||
            // 网络错误（reqwest 错误）
            msg.contains("error trying to connect") ||
            msg.contains("connection") ||
            msg.contains("timeout") ||
            msg.contains("timed out");

        if is_upstream_error {
            AdminServiceError::UpstreamError(msg)
        } else {
            // 3. 默认归类为内部错误（本地验证失败、配置错误等）
            // 包括：缺少 refreshToken、refreshToken 已被截断、无法生成 machineId 等
            AdminServiceError::InternalError(msg)
        }
    }
}
