const path = require('path');
const HtmlWebpackPlugin = require('html-webpack-plugin');
const webpack = require('webpack');
const WasmPackPlugin = require("@wasm-tool/wasm-pack-plugin");
const { SourceMapDevToolPlugin } = require('webpack');

module.exports = {
  entry: './web/src/index.js',
  module: {
    rules: [
      {
        test: /\.tsx?$/,
        use: 'swc-loader',
        exclude: /node_modules/,
      },
      {
        test: /\.scss$/i,
        use: [
          // Creates `style` nodes from JS strings
          "style-loader",
          // Translates CSS into CommonJS
          "css-loader",
          // Compiles Sass to CSS
          "sass-loader",
          // source map for debugging
          "source-map-loader"
        ],
      },
    ],
  },
  resolve: {
    extensions: ['.tsx', '.ts', '.js'],
  },
  output: {
    path: path.resolve(__dirname, 'dist/static'),
    filename: 'index.js',
  },
  plugins: [
    new HtmlWebpackPlugin({
      template: path.resolve(__dirname, 'web/src/index.html'),
      publicPath: "/static",
    }),
    new WasmPackPlugin({
      crateDirectory: path.resolve(__dirname, "web"),
      outDir: path.resolve(__dirname, "web/pkg"),
    }),
    new SourceMapDevToolPlugin({}),
  ],
  experiments: {
    asyncWebAssembly: true,
  },
  mode: 'development'
};
