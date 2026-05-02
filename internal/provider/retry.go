package provider

import (
	"fmt"
	"math"
	"net/http"
	"time"
)

// RetryPolicy describes how to retry HTTP requests.
type RetryPolicy struct {
	// MaxRetries is the maximum number of retry attempts for rate limits.
	MaxRetries int
	// InitialBackoff is the starting delay between retries.
	InitialBackoff time.Duration
	// MaxBackoff caps the delay between retries.
	MaxBackoff time.Duration
	// Retry5xx controls whether 5xx errors get a single retry.
	Retry5xx bool
}

// DefaultRetryPolicy returns the standard policy from the spec:
// 429: up to 4 retries with exponential backoff starting at 1s, capped at 30s.
// 5xx: one retry.
func DefaultRetryPolicy() RetryPolicy {
	return RetryPolicy{
		MaxRetries:     4,
		InitialBackoff: 1 * time.Second,
		MaxBackoff:     30 * time.Second,
		Retry5xx:       true,
	}
}

// ShouldRetry reports whether a given HTTP status warrants a retry.
// It returns the recommended backoff duration and true if retry is advised.
func (p RetryPolicy) ShouldRetry(statusCode int, attempt int) (time.Duration, bool) {
	switch {
	case statusCode == http.StatusTooManyRequests:
		if attempt >= p.MaxRetries {
			return 0, false
		}
		backoff := p.InitialBackoff * time.Duration(math.Pow(2, float64(attempt)))
		if backoff > p.MaxBackoff {
			backoff = p.MaxBackoff
		}
		return backoff, true
	case statusCode >= 500 && statusCode < 600:
		if !p.Retry5xx || attempt >= 1 {
			return 0, false
		}
		return p.InitialBackoff, true
	default:
		return 0, false
	}
}

// ClassifyError maps an HTTP status code to a canonical error kind and message.
func ClassifyError(statusCode int, body string) (kind string, msg string) {
	switch {
	case statusCode == http.StatusUnauthorized || statusCode == http.StatusForbidden:
		return "provider_auth", fmt.Sprintf("authentication failed: %s", body)
	case statusCode == http.StatusTooManyRequests:
		return "provider_rate_limit", fmt.Sprintf("rate limited: %s", body)
	case statusCode >= 500 && statusCode < 600:
		return "provider_overload", fmt.Sprintf("server error (%d): %s", statusCode, body)
	case statusCode >= 400 && statusCode < 500:
		return "provider_api", fmt.Sprintf("client error (%d): %s", statusCode, body)
	default:
		return "unknown", fmt.Sprintf("http %d: %s", statusCode, body)
	}
}
