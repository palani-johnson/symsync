from cyclopts import App

app = App()


@app.command
def config():
  print("config")


@app.default
def main():
  print("main")


if __name__ == "__main__":
  app()
