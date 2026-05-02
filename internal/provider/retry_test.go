package provider

import (
	"net/http"
	"testing"
	"time"
)

func TestShouldRetry429(t *testing.T) {
	p := DefaultRetryPolicy()

	tests := []struct {
		attempt int
		want    time.Duration
		ok      bool
	}{
		{0, 1 * time.Second, true},
		{1, 2 * time.Second, true},
		{2, 4 * time.Second, true},
		{3, 8 * time.Second, true},
		{4, 0, false},
	}

	for _, tt := range tests {
		backoff, ok := p.ShouldRetry(http.StatusTooManyRequests, tt.attempt)
		if ok != tt.ok {
			t.Errorf("attempt %d: ok = %v, want %v", tt.attempt, ok, tt.ok)
		}
		if ok && backoff != tt.want {
			t.Errorf("attempt %d: backoff = %v, want %v", tt.attempt, backoff, tt.want)
		}
	}
}

func TestShouldRetry429Capped(t *testing.T) {
	p := RetryPolicy{
		MaxRetries:     10,
		InitialBackoff: 10 * time.Second,
		MaxBackoff:     30 * time.Second,
	}

	backoff, ok := p.ShouldRetry(http.StatusTooManyRequests, 2)
	if !ok {
		t.Fatal("expected retry")
	}
	// 10s * 2^2 = 40s, but capped at 30s.
	if backoff != 30*time.Second {
		t.Errorf("backoff = %v, want 30s", backoff)
	}
}

func TestShouldRetry5xx(t *testing.T) {
	p := DefaultRetryPolicy()

	backoff, ok := p.ShouldRetry(http.StatusInternalServerError, 0)
	if !ok {
		t.Fatal("expected retry on 5xx")
	}
	if backoff != p.InitialBackoff {
		t.Errorf("backoff = %v, want %v", backoff, p.InitialBackoff)
	}

	_, ok = p.ShouldRetry(http.StatusInternalServerError, 1)
	if ok {
		t.Error("expected no retry on second 5xx attempt")
	}
}

func TestShouldRetryNoRetry5xx(t *testing.T) {
	p := RetryPolicy{Retry5xx: false}
	_, ok := p.ShouldRetry(http.StatusInternalServerError, 0)
	if ok {
		t.Error("expected no retry when Retry5xx is false")
	}
}

func TestShouldRetry4xx(t *testing.T) {
	p := DefaultRetryPolicy()
	_, ok := p.ShouldRetry(http.StatusBadRequest, 0)
	if ok {
		t.Error("expected no retry on 4xx")
	}
}

func TestClassifyError(t *testing.T) {
	tests := []struct {
		code     int
		wantKind string
	}{
		{http.StatusUnauthorized, "provider_auth"},
		{http.StatusForbidden, "provider_auth"},
		{http.StatusTooManyRequests, "provider_rate_limit"},
		{http.StatusInternalServerError, "provider_overload"},
		{http.StatusBadGateway, "provider_overload"},
		{http.StatusBadRequest, "provider_api"},
		{http.StatusNotFound, "provider_api"},
		{999, "unknown"},
	}
	for _, tt := range tests {
		kind, _ := ClassifyError(tt.code, "body")
		if kind != tt.wantKind {
			t.Errorf("ClassifyError(%d) kind = %q, want %q", tt.code, kind, tt.wantKind)
		}
	}
}
