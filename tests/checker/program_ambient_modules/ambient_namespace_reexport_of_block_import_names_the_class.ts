// @strict: true
// @module: nodenext
// @filename: types/events.d.ts
declare module "events" {
  class EventEmitter {
    on(event: string, listener: () => void): this;
  }
  import internal = require("node:events");
  namespace EventEmitter {
    export { internal as EventEmitter };
  }
  export = EventEmitter;
}
declare module "node:events" {
  import events = require("events");
  export = events;
}
// @filename: index.ts
/// <reference path="./types/events.d.ts" />
import events = require("events");
declare const emitter: events.EventEmitter;
emitter.on("x", () => {});
emitter.missing;
class Client extends events.EventEmitter {}
declare const client: Client;
client.missing;
