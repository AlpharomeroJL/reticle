//! Rate-limited demo server for Reticle.
//!
//! An axum service implementing the frozen submit, status, and cancel endpoints
//! with all limits mandatory: per-IP rate and concurrency, a global cap, token
//! and command budgets, a maximum prompt length, and a task-vocabulary input
//! filter, so the demo is safe to expose publicly. Cancel kills the session.
//! Frozen Wave 0 skeleton; the service and limit enforcement land in a later wave.
