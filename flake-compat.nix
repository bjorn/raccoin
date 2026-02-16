let
  src = fetchTarball "https://github.com/edolstra/flake-compat/archive/master.tar.gz";
in
import src { src = ./.; }
