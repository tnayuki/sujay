import axios from "axios";
import type { AxiosInstance } from "axios";
import UserAgent from "user-agents";
import * as cookie from "cookie";
import { randomUUID } from "node:crypto";

// Simple sleep utility
const sleep = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms));

export interface AudioInfo {
  id: string; // Unique identifier for the audio
  title?: string; // Title of the audio
  image_url?: string; // URL of the image associated with the audio
  lyric?: string; // Lyrics of the audio
  audio_url?: string; // URL of the audio file
  video_url?: string; // URL of the video associated with the audio
  created_at: string; // Date and time when the audio was created
  model_name: string; // Name of the model used for audio generation
  gpt_description_prompt?: string; // Prompt for GPT description
  prompt?: string; // Prompt for audio generation
  status: string; // Status
  type?: string;
  tags?: string; // Genre of music.
  negative_tags?: string; // Negative tags of music.
  duration?: string; // Duration of the audio
  error_message?: string; // Error message if any
  metadata_tags?: string; // Additional metadata tags
  is_liked?: boolean; // Whether the track is liked
}

class SunoApi {
  private static BASE_URL = "https://studio-api.prod.suno.com";
  private static CLERK_BASE_URL = "https://clerk.suno.com";
  private static CLERK_VERSION = "5.15.0";

  private readonly client: AxiosInstance;
  private sid?: string;
  private currentToken?: string;
  private deviceId?: string;
  private userAgent?: string;
  private cookies: Record<string, string | undefined>;

  constructor(cookies: string) {
    this.userAgent = new UserAgent(/Macintosh/).random().toString();
    this.cookies = cookie.parse(cookies);
    this.deviceId = this.cookies.ajs_anonymous_id || randomUUID();
    this.client = axios.create({
      withCredentials: true,
      headers: {
        "Affiliate-Id": "undefined",
        "Device-Id": `"${this.deviceId}"`,
        "x-suno-client": "Android prerelease-4nt180t 1.0.42",
        "X-Requested-With": "com.suno.android",
        "sec-ch-ua":
          '"Chromium";v="130", "Android WebView";v="130", "Not?A_Brand";v="99"',
        "sec-ch-ua-mobile": "?1",
        "sec-ch-ua-platform": '"Android"',
        "User-Agent": this.userAgent,
      },
    });

    this.client.interceptors.request.use((config) => {
      if (this.currentToken && !config.headers.Authorization)
        config.headers.Authorization = `Bearer ${this.currentToken}`;
      const cookiesArray = Object.entries(this.cookies).map(([key, value]) =>
        cookie.serialize(key, value as string)
      );
      config.headers.Cookie = cookiesArray.join("; ");
      return config;
    });

    this.client.interceptors.response.use((resp) => {
      const setCookieHeader = resp.headers["set-cookie"];
      if (Array.isArray(setCookieHeader)) {
        const newCookies = cookie.parse(setCookieHeader.join("; "));
        for (const [key, value] of Object.entries(newCookies)) {
          this.cookies[key] = value;
        }
      }
      return resp;
    });
  }

  public async init(): Promise<SunoApi> {
    await this.getAuthToken();
    await this.keepAlive();

    return this;
  }

  /**
   * Get the session ID and save it for later use.
   */
  private async getAuthToken() {
    // URL to get session ID
    const getSessionUrl = `${SunoApi.CLERK_BASE_URL}/v1/client?_is_native=true&_clerk_js_version=${SunoApi.CLERK_VERSION}`;

    // Get session ID
    const sessionResponse = await this.client.get(getSessionUrl, {
      headers: { Authorization: this.cookies.__client },
    });

    if (!sessionResponse?.data?.response?.last_active_session_id) {
      throw new Error(
        "Failed to get session id, you may need to update the SUNO_COOKIE"
      );
    }

    // Save session ID for later use
    this.sid = sessionResponse.data.response.last_active_session_id;
  }

  /**
   * Keep the session alive.
   * @param isWait Indicates if the method should wait for the session to be fully renewed before returning.
   */
  private isTokenExpired(): boolean {
    if (!this.currentToken) return true;

    try {
      // JWT token is in format: header.payload.signature
      const parts = this.currentToken.split(".");
      if (parts.length !== 3) return true;

      const payload = parts[1];
      const decoded = JSON.parse(Buffer.from(payload, "base64").toString());

      // Check if token expires within 5 minutes
      const exp = decoded.exp * 1000; // Convert to milliseconds
      const now = Date.now();
      const fiveMinutes = 5 * 60 * 1000;

      return exp - now < fiveMinutes;
    } catch (e) {
      return true; // If we can't decode, assume expired
    }
  }

  public async keepAlive(isWait?: boolean): Promise<void> {
    if (!this.sid) {
      throw new Error("Session ID is not set. Cannot renew token.");
    }

    // Only renew if token is expired or about to expire
    if (!this.isTokenExpired()) {
      return;
    }

    // URL to renew session token
    const renewUrl = `${SunoApi.CLERK_BASE_URL}/v1/client/sessions/${this.sid}/tokens?_is_native=true&_clerk_js_version=${SunoApi.CLERK_VERSION}`;

    // Renew session token
    const renewResponse = await this.client.post(
      renewUrl,
      {},
      {
        headers: { Authorization: this.cookies.__client },
      }
    );

    if (isWait) {
      await sleep(1000);
    }

    const newToken = renewResponse.data.jwt;

    // Update Authorization field in request header with the new JWT token
    this.currentToken = newToken;
  }

  /**
   * Get workspaces (projects) from Suno
   * @param page Page number (starting from 1)
   * @returns Array of workspace objects
   */
  public async getWorkspaces(page = 1): Promise<Record<string, unknown>[]> {
    await this.keepAlive(false);
    const url = `${SunoApi.BASE_URL}/api/project/me?page=${page}`;
    const response = await this.client.get(url, {
      timeout: 10000,
    });
    return response.data.projects || [];
  }

  /**
   * Get feed using v3 API with workspace and liked filters
   * @param workspaceId Workspace ID (use "default" for default workspace, null for all)
   * @param cursor Cursor for pagination (null for first page)
   * @param limit Number of items to fetch (default 20)
   * @param liked Filter for liked songs only (default false)
   * @returns Feed response with clips and next cursor
   */
  public async getFeedV3(
    workspaceId: string | null = null,
    cursor: string | null = null,
    limit = 20,
    liked = false
  ): Promise<{ clips: AudioInfo[]; cursor: string | null }> {
    await this.keepAlive(false);

    const payload: Record<string, unknown> = {
      cursor: cursor,
      limit: limit,
      filters: {
        disliked: "False",
        trashed: "False",
        fromStudioProject: {
          presence: "False",
        },
        stem: {
          presence: "False",
        },
      },
    };

    // Add liked filter if specified
    if (liked) {
      (payload.filters as Record<string, unknown>).liked = "True";
    }

    // Add workspace filter if specified
    if (workspaceId) {
      (payload.filters as Record<string, unknown>).workspace = {
        workspaceId: workspaceId,
        presence: "True",
      };
    }

    const response = await this.client.post(
      `${SunoApi.BASE_URL}/api/feed/v3`,
      payload,
      {
        timeout: 10000,
      }
    );

    const clips = response.data.clips.map((audio: AudioInfo & { metadata?: Record<string, unknown> }) => ({
      id: audio.id,
      title: audio.title,
      image_url: audio.image_url,
      lyric: typeof audio.metadata?.prompt === 'string' ? audio.metadata.prompt : undefined,
      audio_url: audio.audio_url,
      video_url: audio.video_url,
      created_at: audio.created_at,
      model_name: audio.model_name,
      status: audio.status,
      gpt_description_prompt: typeof audio.metadata?.gpt_description_prompt === 'string' ? audio.metadata.gpt_description_prompt : undefined,
      prompt: typeof audio.metadata?.prompt === 'string' ? audio.metadata.prompt : undefined,
      type: typeof audio.metadata?.type === 'string' ? audio.metadata.type : undefined,
      tags: typeof audio.metadata?.tags === 'string' ? audio.metadata.tags : undefined,
      negative_tags: typeof audio.metadata?.negative_tags === 'string' ? audio.metadata.negative_tags : undefined,
      duration: typeof audio.metadata?.duration === 'string' || typeof audio.metadata?.duration === 'number' ? String(audio.metadata.duration) : undefined,
      metadata_tags: typeof audio.metadata?.metadata_tags === 'string' ? audio.metadata.metadata_tags : undefined,
      is_liked: audio.is_liked,
      error_message: typeof audio.metadata?.error_message === 'string' ? audio.metadata.error_message : undefined,
    }));

    return {
      clips: clips,
      cursor: response.data.next_cursor || null,
    };
  }

  public async get(
    songIds?: string[],
    page?: string | null
  ): Promise<AudioInfo[]> {
    await this.keepAlive(false);
    const url = new URL(`${SunoApi.BASE_URL}/api/feed/v2`);
    if (songIds) {
      url.searchParams.append("ids", songIds.join(","));
    }
    if (page) {
      url.searchParams.append("page", page);
    }
    // logger.info("Get audio status: " + url.href);
    const response = await this.client.get(url.href, {
      // 10 seconds timeout
      timeout: 10000,
    });

    const audios = response.data.clips;

    return audios.map((audio: AudioInfo & { metadata?: Record<string, unknown> }) => ({
      id: audio.id,
      title: audio.title,
      image_url: audio.image_url,
      lyric: typeof audio.metadata?.prompt === 'string' ? audio.metadata.prompt : undefined,
      audio_url: audio.audio_url,
      video_url: audio.video_url,
      created_at: audio.created_at,
      model_name: audio.model_name,
      status: audio.status,
      gpt_description_prompt: typeof audio.metadata?.gpt_description_prompt === 'string' ? audio.metadata.gpt_description_prompt : undefined,
      prompt: typeof audio.metadata?.prompt === 'string' ? audio.metadata.prompt : undefined,
      type: typeof audio.metadata?.type === 'string' ? audio.metadata.type : undefined,
      tags: typeof audio.metadata?.tags === 'string' ? audio.metadata.tags : undefined,
      duration: typeof audio.metadata?.duration === 'string' || typeof audio.metadata?.duration === 'number' ? String(audio.metadata.duration) : undefined,
      is_liked: audio.is_liked,
      error_message: typeof audio.metadata?.error_message === 'string' ? audio.metadata.error_message : undefined,
    }));
  }
}

/**
 * Helper function to create and initialize a SunoApi instance
 * @param cookie Suno session cookie
 * @returns Initialized SunoApi client
 */
export async function sunoApi(cookie: string): Promise<SunoApi> {
  const client = new SunoApi(cookie);
  await client.init();
  return client;
}
