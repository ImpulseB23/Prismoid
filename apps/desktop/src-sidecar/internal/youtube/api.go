package youtube

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"unicode/utf8"
)

const defaultDataAPIBase = "https://youtube.googleapis.com/youtube/v3"

// MaxMessageRunes is the documented upper bound on liveChat message text
// length (200 Unicode characters per Google's API reference). Mirrored
// here so we reject oversize payloads before consuming a quota unit.
const MaxMessageRunes = 200

// ErrUnauthorized is returned (wrapped inside [APIError]) when the
// YouTube Data API responds with 401. Callers use
// `errors.Is(err, ErrUnauthorized)` to decide whether to refresh the
// access token.
var ErrUnauthorized = errors.New("youtube api: unauthorized")

// ErrQuotaExceeded is returned (wrapped inside [APIError]) when the
// API responds with 403 + reason `quotaExceeded` or 429. The Rust host
// surfaces this back to the UI so the user knows the daily Data API
// quota was hit rather than a transient failure.
var ErrQuotaExceeded = errors.New("youtube api: quota exceeded")

// APIError is the structured error envelope Google returns for Data API
// failures. Shape: `{"error": {"code": int, "message": string, "errors":
// [{"reason": string, ...}]}}`.
type APIError struct {
	Status  int    `json:"-"`
	Code    int    `json:"code"`
	Message string `json:"message"`
	Reason  string `json:"-"`
}

func (e *APIError) Error() string {
	if e.Reason != "" {
		return fmt.Sprintf("youtube api %d (%s): %s", e.Status, e.Reason, e.Message)
	}
	return fmt.Sprintf("youtube api %d: %s", e.Status, e.Message)
}

// Is lets `errors.Is(err, ErrUnauthorized)` / `errors.Is(err, ErrQuotaExceeded)`
// succeed regardless of how the APIError is wrapped.
func (e *APIError) Is(target error) bool {
	switch target {
	case ErrUnauthorized:
		return e.Status == http.StatusUnauthorized
	case ErrQuotaExceeded:
		return e.Status == http.StatusTooManyRequests ||
			(e.Status == http.StatusForbidden && e.Reason == "quotaExceeded")
	}
	return false
}

// APIClient is a thin REST client for the YouTube Data API v3. Scope is
// deliberately narrow — write operations only. Reads go through the
// gRPC streaming client in the same package.
type APIClient struct {
	HTTPClient  *http.Client
	BaseURL     string
	AccessToken string
}

// SendChatMessageRequest is the body shape POST /liveChat/messages
// expects. We only support text messages; super chats / poll votes
// have separate dedicated endpoints in the broader API.
type SendChatMessageRequest struct {
	Snippet sendChatSnippet `json:"snippet"`
}

type sendChatSnippet struct {
	LiveChatID         string             `json:"liveChatId"`
	Type               string             `json:"type"`
	TextMessageDetails sendChatTextDetail `json:"textMessageDetails"`
}

type sendChatTextDetail struct {
	MessageText string `json:"messageText"`
}

// SendChatMessageResponse mirrors Google's liveChatMessage resource
// envelope. We only surface the assigned `id` to the host; the rest of
// the resource is echoed back over the gRPC stream as an authoritative
// message anyway.
type SendChatMessageResponse struct {
	ID string `json:"id"`
}

// SendChatMessage posts a text message to the supplied liveChatId via
// the YouTube Data API and returns the assigned message id. Validation
// (empty / too long) runs before the request so we don't burn quota
// units on locally-rejectable input.
func (c *APIClient) SendChatMessage(ctx context.Context, liveChatID, message string) (*SendChatMessageResponse, error) {
	if liveChatID == "" {
		return nil, errors.New("youtube api: missing liveChatId")
	}
	if message == "" {
		return nil, errors.New("youtube api: empty chat message")
	}
	if utf8.RuneCountInString(message) > MaxMessageRunes {
		return nil, fmt.Errorf("youtube api: chat message exceeds %d characters", MaxMessageRunes)
	}

	req := SendChatMessageRequest{
		Snippet: sendChatSnippet{
			LiveChatID:         liveChatID,
			Type:               "textMessageEvent",
			TextMessageDetails: sendChatTextDetail{MessageText: message},
		},
	}

	var resp SendChatMessageResponse
	if err := c.do(ctx, http.MethodPost, "/liveChat/messages?part=snippet", req, &resp); err != nil {
		return nil, err
	}
	return &resp, nil
}

func (c *APIClient) do(ctx context.Context, method, path string, reqBody, dst any) error {
	base := c.BaseURL
	if base == "" {
		base = defaultDataAPIBase
	}

	var body io.Reader
	if reqBody != nil {
		buf, err := json.Marshal(reqBody)
		if err != nil {
			return fmt.Errorf("marshal youtube request body: %w", err)
		}
		body = bytes.NewReader(buf)
	}

	httpReq, err := http.NewRequestWithContext(ctx, method, base+path, body)
	if err != nil {
		return fmt.Errorf("build youtube request: %w", err)
	}
	httpReq.Header.Set("Authorization", "Bearer "+c.AccessToken)
	if reqBody != nil {
		httpReq.Header.Set("Content-Type", "application/json")
	}

	httpClient := c.HTTPClient
	if httpClient == nil {
		httpClient = http.DefaultClient
	}

	httpResp, err := httpClient.Do(httpReq)
	if err != nil {
		return fmt.Errorf("youtube request: %w", err)
	}
	defer func() { _ = httpResp.Body.Close() }()

	if httpResp.StatusCode < 200 || httpResp.StatusCode >= 300 {
		buf, _ := io.ReadAll(httpResp.Body)
		return decodeAPIError(httpResp.StatusCode, buf)
	}

	if dst == nil {
		_, _ = io.Copy(io.Discard, httpResp.Body)
		return nil
	}
	if err := json.NewDecoder(httpResp.Body).Decode(dst); err != nil {
		return fmt.Errorf("decode youtube response: %w", err)
	}
	return nil
}

// decodeAPIError parses the Google JSON error envelope. A non-JSON body
// still surfaces via [APIError.Message] so logs carry the raw response.
func decodeAPIError(status int, body []byte) error {
	type errItem struct {
		Reason string `json:"reason"`
	}
	type envelope struct {
		Error struct {
			Code    int       `json:"code"`
			Message string    `json:"message"`
			Errors  []errItem `json:"errors"`
		} `json:"error"`
	}
	apiErr := &APIError{Status: status}
	var env envelope
	if json.Unmarshal(body, &env) == nil && env.Error.Message != "" {
		apiErr.Code = env.Error.Code
		apiErr.Message = env.Error.Message
		if len(env.Error.Errors) > 0 {
			apiErr.Reason = env.Error.Errors[0].Reason
		}
	} else {
		apiErr.Message = string(body)
	}
	return apiErr
}
