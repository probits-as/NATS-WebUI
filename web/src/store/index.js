import Vue from 'vue'
import Vuex from 'vuex'
import axios from 'axios'
import VueAxios from 'vue-axios'
import VueRouter from 'vue-router'

var Deque = require('double-ended-queue')

Vue.use(Vuex)
Vue.use(VueRouter)
Vue.use(VueAxios, axios)

export default new Vuex.Store({
  state: {
    app_state: {
      servers: [],
      clients: []
    },
    transient: {
      serversMap: {},
      clientsMap: {},
      selectedServer: 1,
      selectedClient: 1,
      server: true,
      index: -1
    }
  },
  getters: {},
  mutations: {
    updateAppState(state, appState) {
      appState.clients.forEach(function (c) {
        c.messages = []
        c.messageBuffer = new Deque(100)
        c.socket = null
      })
      state.app_state = appState
      state.transient.serversMap = state.app_state.servers.reduce((o, e) => { o[e.id] = e; return o }, {})
      state.transient.clientsMap = state.app_state.clients.reduce((o, e) => { o[e.id] = e; return o }, {})
    },
    selectScreen(state, { isServer, index }) {
      state.transient.server = isServer
      state.transient.index = index
      if (isServer) {
        state.transient.selectedServer = index
      } else {
        state.transient.selectedClient = index
      }
    },
    updateServerSubjects(state, { serverId, subjects }) {
      const server = state.transient.serversMap[serverId];
      if (server) {
        Vue.set(server, 'subjects', subjects);
      }
    }
  },
  actions: {
    async getAppState(ctx) {
      var resp = await axios.get('/api/state')
      ctx.commit('updateAppState', resp.data)
    },
    async createServer(ctx, server) {
      await axios.post('/api/state/server/new', server)
      await ctx.dispatch('getAppState')
    },
    async updateServer(ctx, server) {
      await axios.post('/api/state/server/update', server)
      await ctx.dispatch('getAppState')
    },
    async deleteServer(ctx, server_id) {
      await axios.get('/api/state/server/delete/' + server_id)
      await ctx.dispatch('getAppState')
    },
    async createClient(ctx, client) {
      await axios.post('/api/state/client/new', client)
      await ctx.dispatch('getAppState')
    },
    async updateClient(ctx, client) {
      await axios.post('/api/state/client/update', client)
      await ctx.dispatch('getAppState')
    },
    async deleteClient(ctx, client_id) {
      await axios.get('/api/state/client/delete/' + client_id)
      await ctx.dispatch('getAppState')
    },
    async fetchServerSubjects(ctx, serverId) {
      const response = await axios.get(`/api/state/server/${serverId}/subjects`);
      const newSubjects = response.data;
      const currentSubjects = ctx.state.transient.serversMap[serverId].subjects || [];

      // Merge new subjects with existing user-added subjects
      const mergedSubjects = mergeSubjects(currentSubjects, newSubjects);

      ctx.commit('updateServerSubjects', { serverId, subjects: mergedSubjects });
      return mergedSubjects;
    },
  }
})

function mergeSubjects(currentSubjects, newSubjects) {
  const mergedSubjects = [...currentSubjects];

  newSubjects.forEach(newSubject => {
    const existingIndex = mergedSubjects.findIndex(s => s.id === newSubject.id);
    if (existingIndex === -1) {
      mergedSubjects.push(newSubject);
    } else if (mergedSubjects[existingIndex].source !== 'User') {
      mergedSubjects[existingIndex] = newSubject;
    }
  });

  // Remove server-populated subjects that no longer exist
  return mergedSubjects.filter(subject =>
    subject.source === 'User' || newSubjects.some(s => s.id === subject.id)
  );
}
