package twitch

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
)

const defaultHelixBase = "https://api.twitch.tv/helix"

// Subscribe creates an EventSub subscription for channel.chat.message via the Helix API.
// helixBase can be overridden for testing; pass "" to use the default.
func Subscribe(ctx context.Context, helixBase, sessionID, broadcasterID, userID, accessToken, clientID string) error {
	if helixBase == "" {
		helixBase = defaultHelixBase
	}

	body, err := json.Marshal(SubscriptionRequest{
		Type:    "channel.chat.message",
		Version: "1",
		Condition: map[string]string{
			"broadcaster_user_id": broadcasterID,
			"user_id":             userID,
		},
		Transport: Transport{
			Method:    "websocket",
			SessionID: sessionID,
		},
	})
	if err != nil {
		return fmt.Errorf("marshal subscription request: %w", err)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, helixBase+"/eventsub/subscriptions", bytes.NewReader(body))
	if err != nil {
		return fmt.Errorf("create request: %w", err)
	}
	req.Header.Set("Authorization", "Bearer "+accessToken)
	req.Header.Set("Client-Id", clientID)
	req.Header.Set("Content-Type", "application/json")

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return fmt.Errorf("helix request: %w", err)
	}
	defer func() { _ = resp.Body.Close() }()

	respBody, _ := io.ReadAll(resp.Body)

	if resp.StatusCode == http.StatusUnauthorized {
		return &AuthError{Status: resp.StatusCode, Body: string(respBody)}
	}
	if resp.StatusCode != http.StatusAccepted {
		return fmt.Errorf("helix returned %d: %s", resp.StatusCode, respBody)
	}

	return nil
}

type AuthError struct {
	Status int
	Body   string
}

func (e *AuthError) Error() string {
	return fmt.Sprintf("twitch auth error (%d): %s", e.Status, e.Body)
}
