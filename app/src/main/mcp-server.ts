/**
 * MCP Server for Sujay DJ Application
 */

/* eslint-disable import/no-unresolved */
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
/* eslint-enable import/no-unresolved */
import express from 'express';
import cors from 'cors';
import { MCPController } from '../core/controllers/mcp-controller';
import type { Server as NodeServer } from 'http';

const PORT = process.env.MCP_PORT || 8888;

let nodeServer: NodeServer | null = null;

/**
 * Start MCP server
 */
export async function startMcpServer(controller: MCPController): Promise<void> {
  const server = new Server(
    {
      name: 'sujay-mcp-server',
      version: '0.1.0',
    },
    {
      capabilities: {
        tools: {},
      },
    }
  );

  // Tool definitions
  server.setRequestHandler(ListToolsRequestSchema, async () => {
    return {
      tools: [
        {
          name: 'list_workspaces',
          description: 'List all available Suno workspaces',
          inputSchema: {
            type: 'object' as const,
            properties: {},
            required: [] as string[],
          },
        },
        {
          name: 'get_selected_workspace',
          description: 'Get currently selected workspace',
          inputSchema: {
            type: 'object' as const,
            properties: {},
            required: [] as string[],
          },
        },
        {
          name: 'select_workspace',
          description: 'Select active workspace (null to clear selection)',
          inputSchema: {
            type: 'object' as const,
            properties: {
              workspaceId: {
                type: 'string',
                description: 'Workspace ID (null to clear)',
              },
            },
            required: [] as string[],
          },
        },
        {
          name: 'list_tracks',
          description:
            'List all tracks in current workspace with metadata (is_liked, tags, gpt_description_prompt, etc.)',
          inputSchema: {
            type: 'object' as const,
            properties: {},
            required: [] as string[],
          },
        },
        {
          name: 'load_deck',
          description: 'Load track to deck (downloads if needed)',
          inputSchema: {
            type: 'object' as const,
            properties: {
              trackId: {
                type: 'string',
                description: 'Track ID',
              },
              deck: {
                type: 'number',
                enum: [1, 2],
                description: 'Deck number (1 or 2)',
              },
            },
            required: ['trackId', 'deck'] as string[],
          },
        },
        {
          name: 'play_deck',
          description: 'Start playback on deck',
          inputSchema: {
            type: 'object' as const,
            properties: {
              deck: {
                type: 'number',
                enum: [1, 2],
                description: 'Deck number (1 or 2)',
              },
            },
            required: ['deck'] as string[],
          },
        },
        {
          name: 'stop_deck',
          description: 'Stop playback on deck',
          inputSchema: {
            type: 'object' as const,
            properties: {
              deck: {
                type: 'number',
                enum: [1, 2],
                description: 'Deck number (1 or 2)',
              },
            },
            required: ['deck'] as string[],
          },
        },
        {
          name: 'seek_deck',
          description: 'Seek deck to specific position in seconds',
          inputSchema: {
            type: 'object' as const,
            properties: {
              deck: {
                type: 'number',
                enum: [1, 2],
                description: 'Deck number (1 or 2)',
              },
              position: {
                type: 'number',
                minimum: 0,
                description: 'Position in seconds',
              },
            },
            required: ['deck', 'position'] as string[],
          },
        },
        {
          name: 'set_crossfader',
          description: 'Set manual crossfader position (0 = full A, 1 = full B)',
          inputSchema: {
            type: 'object' as const,
            properties: {
              position: {
                type: 'number',
                minimum: 0,
                maximum: 1,
                description: 'Crossfader position (0-1)',
              },
            },
            required: ['position'] as string[],
          },
        },
        {
          name: 'get_crossfader',
          description:
            'Get crossfader state (position, auto-crossfade status, progress)',
          inputSchema: {
            type: 'object' as const,
            properties: {},
            required: [] as string[],
          },
        },
        {
          name: 'trigger_crossfade',
          description: 'Trigger auto crossfade between currently loaded decks',
          inputSchema: {
            type: 'object' as const,
            properties: {
              targetPosition: {
                type: 'number',
                minimum: 0,
                maximum: 1,
                description: 'Target crossfader position (0 = full deck A, 1 = full deck B). If omitted, automatically determined based on current playing deck.',
              },
              duration: {
                type: 'number',
                minimum: 0.1,
                maximum: 10,
                description: 'Crossfade duration in seconds (default: 2)',
              },
            },
            required: [] as string[],
          },
        },
        {
          name: 'get_deck_info',
          description: 'Get deck information (loaded track, playing status, position, remaining time)',
          inputSchema: {
            type: 'object' as const,
            properties: {
              deck: {
                type: 'number',
                description: 'Deck number (1 or 2)',
                enum: [1, 2],
              },
            },
            required: ['deck'] as string[],
          },
        },
        {
          name: 'set_eq_cut',
          description: 'Set EQ cut (kill) state for a specific frequency band on a deck. When enabled, the specified frequency band is completely removed from the audio.',
          inputSchema: {
            type: 'object' as const,
            properties: {
              deck: {
                type: 'number',
                enum: [1, 2],
                description: 'Deck number (1 or 2)',
              },
              band: {
                type: 'string',
                enum: ['low', 'mid', 'high'],
                description: 'Frequency band (low = bass, mid = midrange, high = treble)',
              },
              enabled: {
                type: 'boolean',
                description: 'Enable (true) or disable (false) the EQ cut',
              },
            },
            required: ['deck', 'band', 'enabled'] as string[],
          },
        },
        {
          name: 'get_eq_state',
          description: 'Get current EQ cut (kill) state for both decks. Returns which frequency bands are currently cut on each deck.',
          inputSchema: {
            type: 'object' as const,
            properties: {},
            required: [] as string[],
          },
        },
        {
          name: 'get_master_tempo',
          description: 'Get current master tempo in BPM. This is the tempo that all decks are synchronized to.',
          inputSchema: {
            type: 'object' as const,
            properties: {},
            required: [] as string[],
          },
        },
        {
          name: 'get_track_structure',
          description: 'Get track structure analysis (intro/outro/main sections) for optimal DJ mixing. Returns BPM, section boundaries (intro end, outro start), beat counts, and hot cues at important positions.',
          inputSchema: {
            type: 'object' as const,
            properties: {
              trackId: {
                type: 'string',
                description: 'Track ID',
              },
            },
            required: ['trackId'] as string[],
          },
        },
        {
          name: 'set_master_tempo',
          description: 'Set master tempo in BPM. All playing decks will be synchronized to this tempo.',
          inputSchema: {
            type: 'object' as const,
            properties: {
              bpm: {
                type: 'number',
                minimum: 60,
                maximum: 200,
                description: 'Master tempo in BPM (60-200)',
              },
            },
            required: ['bpm'] as string[],
          },
        },
        {
          name: 'get_playback_time_remaining',
          description: 'Get remaining playback time in seconds for a deck. Returns null if deck is not playing.',
          inputSchema: {
            type: 'object' as const,
            properties: {
              deck: {
                type: 'number',
                description: 'Deck number (1 or 2)',
                enum: [1, 2],
              },
            },
            required: ['deck'] as string[],
          },
        },
        {
          name: 'wait_until_position',
          description: 'Wait until deck reaches specified position/time. This is a blocking operation. Specify exactly one of: remainingSeconds, positionSeconds, or elapsedSeconds.',
          inputSchema: {
            type: 'object' as const,
            properties: {
              deck: {
                type: 'number',
                description: 'Deck number (1 or 2)',
                enum: [1, 2],
              },
              remainingSeconds: {
                type: 'number',
                description: 'Wait until this many seconds remain in the track',
              },
              positionSeconds: {
                type: 'number',
                description: 'Wait until playback reaches this position (in seconds)',
              },
              elapsedSeconds: {
                type: 'number',
                description: 'Wait until this many seconds have elapsed',
              },
            },
            required: ['deck'] as string[],
          },
        },
      ],
    };
  });

  // Tool call handler
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  server.setRequestHandler(CallToolRequestSchema, async (request: any) => {
    try {
      const { name, arguments: args } = request.params;

      switch (name) {
        case 'list_workspaces': {
          const workspaces = await controller.listWorkspaces();
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(workspaces, null, 2),
              },
            ],
          };
        }

        case 'get_selected_workspace': {
          const workspace = await controller.getSelectedWorkspace();
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(workspace, null, 2),
              },
            ],
          };
        }

        case 'select_workspace': {
          const { workspaceId } = args as { workspaceId?: string };
          await controller.selectWorkspace(workspaceId || null);
          return {
            content: [
              {
                type: 'text',
                text: `Workspace selected: ${workspaceId || 'null'}`,
              },
            ],
          };
        }

        case 'list_tracks': {
          const tracks = await controller.listTracks();
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(tracks, null, 2),
              },
            ],
          };
        }

        case 'load_deck': {
          const { trackId, deck } = args as { trackId: string; deck: 1 | 2 };
          await controller.loadDeck(trackId, deck);
          return {
            content: [
              {
                type: 'text',
                text: `Loaded track ${trackId} to deck ${deck}`,
              },
            ],
          };
        }

        case 'play_deck': {
          const { deck } = args as { deck: 1 | 2 };
          await controller.playDeck(deck);
          return {
            content: [
              {
                type: 'text',
                text: `Playing deck ${deck}`,
              },
            ],
          };
        }

        case 'stop_deck': {
          const { deck } = args as { deck: 1 | 2 };
          await controller.stopDeck(deck);
          return {
            content: [
              {
                type: 'text',
                text: `Stopped deck ${deck}`,
              },
            ],
          };
        }

        case 'seek_deck': {
          const { deck, position } = args as { deck: 1 | 2; position: number };
          await controller.seekDeck(deck, position);
          return {
            content: [
              {
                type: 'text',
                text: `Seeked deck ${deck} to ${position.toFixed(2)}s`,
              },
            ],
          };
        }

        case 'set_crossfader': {
          const { position } = args as { position: number };
          await controller.setCrossfader(position);
          return {
            content: [
              {
                type: 'text',
                text: `Crossfader set to: ${position}`,
              },
            ],
          };
        }

        case 'get_crossfader': {
          const state = await controller.getCrossfader();
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(state, null, 2),
              },
            ],
          };
        }

        case 'trigger_crossfade': {
          const { targetPosition, duration } = args as {
            targetPosition?: number;
            duration?: number;
          };
          await controller.triggerCrossfade(
            targetPosition ?? null,
            duration ?? 2
          );
          const positionText = targetPosition !== undefined ? ` to position ${targetPosition}` : '';
          const durationText = duration !== undefined ? ` over ${duration}s` : '';
          return {
            content: [
              {
                type: 'text',
                text: `Triggered crossfade${positionText}${durationText}`,
              },
            ],
          };
        }

        case 'get_deck_info': {
          const { deck } = args as { deck: 1 | 2 };
          const info = await controller.getDeckInfo(deck);
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(info, null, 2),
              },
            ],
          };
        }

        case 'set_eq_cut': {
          const { deck, band, enabled } = args as {
            deck: 1 | 2;
            band: 'low' | 'mid' | 'high';
            enabled: boolean;
          };
          await controller.setEqCut(deck, band, enabled);
          return {
            content: [
              {
                type: 'text',
                text: `EQ ${band} ${enabled ? 'killed' : 'restored'} on deck ${deck}`,
              },
            ],
          };
        }

        case 'get_eq_state': {
          const state = await controller.getEqState();
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(state, null, 2),
              },
            ],
          };
        }

        case 'get_master_tempo': {
          const tempo = await controller.getMasterTempo();
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({ masterTempo: tempo }, null, 2),
              },
            ],
          };
        }

        case 'get_track_structure': {
          const { trackId } = args as { trackId: string };
          const structure = await controller.getTrackStructure(trackId);
          
          if (!structure) {
            return {
              content: [
                {
                  type: 'text',
                  text: JSON.stringify({ 
                    error: 'Track structure not available. The track may need to be loaded to a deck first for analysis.' 
                  }, null, 2),
                },
              ],
            };
          }

          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({
                  trackId,
                  bpm: structure.bpm,
                  structure: {
                    intro: {
                      start: structure.intro.start,
                      end: structure.intro.end,
                      beats: structure.intro.beats,
                    },
                    main: {
                      start: structure.main.start,
                      end: structure.main.end,
                      beats: structure.main.beats,
                    },
                    outro: {
                      start: structure.outro.start,
                      end: structure.outro.end,
                      beats: structure.outro.beats,
                    },
                  },
                  hotCues: structure.hotCues,
                }, null, 2),
              },
            ],
          };
        }

        case 'set_master_tempo': {
          const { bpm } = args as { bpm: number };
          await controller.setMasterTempo(bpm);
          return {
            content: [
              {
                type: 'text',
                text: `Master tempo set to ${bpm} BPM`,
              },
            ],
          };
        }

        case 'get_playback_time_remaining': {
          const { deck } = args as { deck: 1 | 2 };
          const remaining = await controller.getPlaybackTimeRemaining(deck);
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({ deck, remainingSeconds: remaining }, null, 2),
              },
            ],
          };
        }

        case 'wait_until_position': {
          const { deck, remainingSeconds, positionSeconds, elapsedSeconds } = args as {
            deck: 1 | 2;
            remainingSeconds?: number;
            positionSeconds?: number;
            elapsedSeconds?: number;
          };
          const result = await controller.waitUntilPosition({
            deck,
            remainingSeconds,
            positionSeconds,
            elapsedSeconds,
          });
          
          if (result.reached) {
            return {
              content: [
                {
                  type: 'text',
                  text: `Deck ${deck} reached target position (current: ${result.currentPosition?.toFixed(1)}s, remaining: ${result.remaining?.toFixed(1)}s)`,
                },
              ],
            };
          } else {
            return {
              content: [
                {
                  type: 'text',
                  text: `Timeout (10s) - Deck ${deck} not yet reached target. Current: ${result.currentPosition?.toFixed(1)}s, remaining: ${result.remaining?.toFixed(1)}s. Call again to continue waiting.`,
                },
              ],
            };
          }
        }

        default:
          throw new Error(`Unknown tool: ${name}`);
      }
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      return {
        content: [
          {
            type: 'text',
            text: `Error: ${errorMessage}`,
          },
        ],
        isError: true,
      };
    }
  });

  // Express app with StreamableHTTPServerTransport
  const app = express();
  app.use(cors());
  app.use(express.json());

  app.post('/mcp', async (req, res) => {
    const transport = new StreamableHTTPServerTransport({
      sessionIdGenerator: undefined, // Stateless mode
    });
    await server.connect(transport);
    await transport.handleRequest(req, res, req.body);
  });

  nodeServer = app.listen(PORT, () => {
    console.log(`MCP server listening on http://localhost:${PORT}/mcp`);
  });
}

/**
 * Stop MCP server
 */
export async function stopMcpServer(): Promise<void> {
  if (nodeServer) {
    await new Promise<void>((resolve) => {
      nodeServer.close(() => {
        console.log('MCP server stopped');
        resolve();
      });
    });
    nodeServer = null;
  }
}
