//! Permission checking utilities

use crate::models::TeamMember;

/// Check if a member can perform an action
pub fn check_permission(member: &TeamMember, action: &str) -> bool {
    match action {
        "delete_team" => member.is_owner(),
        "update_team" => member.is_admin_or_owner(),
        "manage_members" => member.can_manage_members(),
        "change_roles" => member.can_change_roles(),
        "share_resources" => member.can_share_resources(),
        "install_resources" => member.can_install_resources(),
        "review_extensions" => member.can_review_extensions(),
        _ => false,
    }
}

/// Permission denied error message
pub fn permission_denied_message(action: &str) -> String {
    format!("You don't have permission to {}", action)
}
