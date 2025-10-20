import Foundation

protocol UserService {
    associatedtype Identifier
    func fetchUser(id: Identifier) async -> String
    func didReceive(user: String)
}

extension UserService {
    func hydrateAndNotify(id: Identifier) async {
        let user = await fetchUser(id: id)
        didReceive(user: user)
    }
}

struct UserAPI: UserService {
    typealias Identifier = Int

    func fetchUser(id: Int) async -> String {
        "user_\(id)"
    }

    func didReceive(user: String) {
        print("received \(user)")
    }

    func mapUsers(ids: [Int]) async -> [String] {
        var output: [String] = []
        for id in ids {
            let user = await fetchUser(id: id)
            output.append(user)
        }
        return output
    }
}
