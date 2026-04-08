import { createSignal } from "solid-js";

export interface ChatMessage {
  id: string;
  platform: "Twitch" | "YouTube" | "Kick";
  timestamp: number;
  arrival_time: number;
  username: string;
  display_name: string;
  platform_user_id: string;
  message_text: string;
  badges: { set_id: string; id: string }[];
  is_mod: boolean;
  is_subscriber: boolean;
  is_broadcaster: boolean;
  color: string | null;
  reply_to: string | null;
}

const MAX_MESSAGES = 5000;
const buffer: ChatMessage[] = [];
const [messages, setMessages] = createSignal<ChatMessage[]>([]);

export function addMessages(batch: ChatMessage[]) {
  buffer.push(...batch);
  if (buffer.length > MAX_MESSAGES) {
    buffer.splice(0, buffer.length - MAX_MESSAGES);
  }
  setMessages([...buffer]);
}

export { messages };
