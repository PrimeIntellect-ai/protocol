FROM node:20-alpine

WORKDIR /app

COPY package*.json ./
RUN npm install
RUN npm install -g ts-node-dev

COPY . .
RUN npm run build

CMD ["npm", "start"]