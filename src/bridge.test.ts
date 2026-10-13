import { beforeEach, describe, expect, it, vi } from "vitest";
const mock = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true, invoke: mock.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mock.listen }));
import { connect } from "./bridge";

describe("native state subscriptions", () => {
  beforeEach(() => { mock.invoke.mockReset(); mock.listen.mockReset(); });
  it("forwards connection changes and releases both subscriptions", async () => {
    const stateCleanup = vi.fn();
    const connectionCleanup = vi.fn();
    mock.listen.mockResolvedValueOnce(stateCleanup).mockResolvedValueOnce(connectionCleanup);
    mock.invoke.mockResolvedValue({});
    const connection = vi.fn();
    const cleanup = await connect(connection);
    const handler = mock.listen.mock.calls[1][1] as (event: { payload: boolean }) => void;
    handler({ payload: false });
    handler({ payload: true });
    expect(connection.mock.calls).toEqual([[false], [true]]);
    cleanup();
    expect(stateCleanup).toHaveBeenCalledTimes(1);
    expect(connectionCleanup).toHaveBeenCalledTimes(1);
  });
  it("releases subscriptions when initial snapshot retrieval fails", async () => {
    const stateCleanup = vi.fn();
    const connectionCleanup = vi.fn();
    mock.listen.mockResolvedValueOnce(stateCleanup).mockResolvedValueOnce(connectionCleanup);
    mock.invoke.mockRejectedValue(new Error("ipc_timeout"));
    await expect(connect()).rejects.toThrow("ipc_timeout");
    expect(stateCleanup).toHaveBeenCalledTimes(1);
    expect(connectionCleanup).toHaveBeenCalledTimes(1);
  });
  it("releases the successful subscription if the other one fails", async () => {
    const cleanup = vi.fn();
    mock.listen.mockResolvedValueOnce(cleanup).mockRejectedValueOnce(new Error("subscription failed"));
    await expect(connect()).rejects.toThrow("subscription failed");
    expect(cleanup).toHaveBeenCalledTimes(1);
    expect(mock.invoke).not.toHaveBeenCalled();
  });
});
